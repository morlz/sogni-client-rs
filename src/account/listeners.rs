use std::{
    future::Future,
    sync::{Arc, Weak},
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{
    AccountApi, AuthProjectionState, CurrentAccount, SubscriptionProjectionState,
    authentication::{apply_current_account, fetch_current_account, reset_account_projection},
    projection::{
        SubscriptionSource, apply_free_spark_projection, apply_subscription_projection,
        event_network, map_socket_subscription, refresh_subscription_projection,
        subscription_version,
    },
};
use crate::{AuthKind, Result, transport::ApiClient};

pub(super) struct AccountListenerLifecycle {
    cancellation: CancellationToken,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    #[cfg(test)]
    probes: Mutex<Vec<Weak<()>>>,
}

impl Default for AccountListenerLifecycle {
    fn default() -> Self {
        Self {
            cancellation: CancellationToken::new(),
            tasks: Mutex::new(Vec::new()),
            #[cfg(test)]
            probes: Mutex::new(Vec::new()),
        }
    }
}

impl AccountListenerLifecycle {
    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        let mut tasks = self.tasks.lock();
        if self.cancellation.is_cancelled() {
            return;
        }
        let cancellation = self.cancellation.clone();
        let task_guard = self.task_guard();
        let task = tokio::spawn(async move {
            let _task_guard = task_guard;
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => {}
                _ = future => {}
            }
        });
        tasks.push(task);
    }

    async fn stop(&self) {
        self.cancellation.cancel();
        let tasks = std::mem::take(&mut *self.tasks.lock());
        for task in tasks {
            if let Err(error) = task.await {
                if !error.is_cancelled() {
                    tracing::debug!(%error, "account listener task stopped unexpectedly");
                }
            }
        }
    }

    fn task_guard(&self) -> ListenerTaskGuard {
        #[cfg(test)]
        {
            let marker = Arc::new(());
            self.probes.lock().push(Arc::downgrade(&marker));
            ListenerTaskGuard { _marker: marker }
        }
        #[cfg(not(test))]
        ListenerTaskGuard {}
    }

    #[cfg(test)]
    fn probes(&self) -> Vec<Weak<()>> {
        self.probes.lock().clone()
    }
}

impl Drop for AccountListenerLifecycle {
    fn drop(&mut self) {
        self.cancellation.cancel();
        for task in self.tasks.get_mut().drain(..) {
            task.abort();
        }
    }
}

struct ListenerTaskGuard {
    #[cfg(test)]
    _marker: Arc<()>,
}

#[derive(Clone)]
struct AccountListenerContext {
    current: CurrentAccount,
    auth_projection: Arc<tokio::sync::Mutex<AuthProjectionState>>,
    subscription_projection: Arc<Mutex<SubscriptionProjectionState>>,
}

impl AccountListenerContext {
    async fn handle_authenticated(&self, client: &ApiClient) -> Result<()> {
        let mut projection = self.auth_projection.lock().await;
        if projection.skip_next_authenticated_update {
            projection.skip_next_authenticated_update = false;
            return Ok(());
        }
        if projection.hydrated || !client.is_authenticated() {
            return Ok(());
        }
        let payload = fetch_current_account(client).await?;
        if !client.is_authenticated() {
            return Ok(());
        }
        apply_current_account(&self.current, &payload);
        projection.hydrated = true;
        Ok(())
    }

    async fn handle_deauthenticated(&self, client: &ApiClient) {
        let mut projection = self.auth_projection.lock().await;
        if client.is_authenticated() {
            return;
        }
        projection.hydrated = false;
        projection.skip_next_authenticated_update = false;
        reset_account_projection(&self.current, &self.subscription_projection);
    }
}

impl AccountApi {
    pub(super) fn listen(&self) {
        self.listen_socket_events();
        self.listen_auth_updates();
    }

    pub(crate) async fn stop_listeners(&self) {
        self.listener_lifecycle.stop().await;
    }

    fn listener_context(&self) -> AccountListenerContext {
        AccountListenerContext {
            current: self.current.clone(),
            auth_projection: self.auth_projection.clone(),
            subscription_projection: self.subscription_projection.clone(),
        }
    }

    fn listen_socket_events(&self) {
        let mut events = self.client.subscribe();
        let client = Arc::downgrade(&self.client);
        let lifecycle = Arc::downgrade(&self.listener_lifecycle);
        let context = self.listener_context();
        let api_key_auth = self.client.auth_kind() == AuthKind::ApiKey;
        self.listener_lifecycle.spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                };
                match event.name.as_str() {
                    "balanceUpdate" if event.data.is_object() => {
                        context.current.update(json!({"balance": event.data}));
                    }
                    "connecting" => context.current.update(json!({
                        "networkStatus": "connecting",
                        "network": event_network(&event.data),
                    })),
                    "connected" | "changeNetwork" => context.current.update(json!({
                        "networkStatus": "connected",
                        "network": event_network(&event.data),
                    })),
                    "disconnected" => context.current.update(json!({
                        "networkStatus": "disconnected",
                        "network": Value::Null,
                    })),
                    "authenticated" if event.data.is_object() => {
                        handle_socket_authenticated(
                            &client,
                            &lifecycle,
                            &context,
                            api_key_auth,
                            &event.data,
                        );
                    }
                    "subscriptionEntitlementUpdated" if event.data.is_object() => {
                        apply_free_spark_projection(&context.current, &event.data);
                        if let Some(mapped) = map_socket_subscription(&event.data) {
                            apply_subscription_projection(
                                &context.current,
                                &context.subscription_projection,
                                mapped,
                                SubscriptionSource::Socket,
                                subscription_version(&event.data),
                                None,
                            );
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    fn listen_auth_updates(&self) {
        let mut updates = self.client.subscribe_auth();
        let client = Arc::downgrade(&self.client);
        let context = self.listener_context();
        let hydrate_initial = self.client.auth_kind() != AuthKind::Token;
        self.listener_lifecycle.spawn(async move {
            let initially_authenticated = *updates.borrow_and_update();
            if initially_authenticated && hydrate_initial {
                let Some(client) = client.upgrade() else {
                    return;
                };
                log_hydration_error(context.handle_authenticated(&client).await);
            }
            while updates.changed().await.is_ok() {
                let authenticated = *updates.borrow_and_update();
                let Some(client) = client.upgrade() else {
                    return;
                };
                if authenticated {
                    log_hydration_error(context.handle_authenticated(&client).await);
                } else {
                    context.handle_deauthenticated(&client).await;
                }
            }
        });
    }

    #[cfg(test)]
    pub(super) fn listener_task_probes(&self) -> Vec<Weak<()>> {
        self.listener_lifecycle.probes()
    }

    #[cfg(test)]
    pub(super) fn client_retention_probe(&self) -> Weak<ApiClient> {
        Arc::downgrade(&self.client)
    }
}

fn handle_socket_authenticated(
    client: &Weak<ApiClient>,
    lifecycle: &Weak<AccountListenerLifecycle>,
    context: &AccountListenerContext,
    api_key_auth: bool,
    data: &Value,
) {
    if api_key_auth {
        context.current.update(json!({
            "username": data.get("username"),
            "walletAddress": data.get("address"),
        }));
    }
    apply_free_spark_projection(&context.current, data);
    if let Some(entitlement) = data.get("subscriptionEntitlement") {
        if let Some(mapped) = map_socket_subscription(entitlement) {
            apply_subscription_projection(
                &context.current,
                &context.subscription_projection,
                mapped,
                SubscriptionSource::Socket,
                subscription_version(entitlement),
                None,
            );
        }
        return;
    }
    let Some(client) = client.upgrade() else {
        return;
    };
    let Some(lifecycle) = lifecycle.upgrade() else {
        return;
    };
    let current = context.current.clone();
    let projection = context.subscription_projection.clone();
    lifecycle.spawn(async move {
        if refresh_subscription_projection(&client, &current, &projection)
            .await
            .is_err()
        {
            tracing::debug!("failed to refresh subscription after authentication");
        }
    });
}

fn log_hydration_error(result: Result<()>) {
    if result.is_err() {
        tracing::debug!("failed to hydrate account after authentication");
    }
}
