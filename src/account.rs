mod authentication;
mod current;
mod listeners;
mod projection;
mod subscriptions;
mod wallet_api;
#[cfg(feature = "wallet")]
mod wallet_crypto;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

use self::listeners::AccountListenerLifecycle;
use crate::transport::ApiClient;

pub use current::CurrentAccount;

#[derive(Clone)]
pub struct AccountApi {
    client: Arc<ApiClient>,
    current: CurrentAccount,
    auth_projection: Arc<AsyncMutex<AuthProjectionState>>,
    subscription_projection: Arc<Mutex<SubscriptionProjectionState>>,
    listener_lifecycle: Arc<AccountListenerLifecycle>,
    testnet: bool,
}

#[derive(Default)]
struct AuthProjectionState {
    hydrated: bool,
    skip_next_authenticated_update: bool,
}

#[derive(Default)]
struct SubscriptionProjectionState {
    last_version: Option<f64>,
    socket_writes: u64,
}

impl std::fmt::Debug for AccountApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountApi")
            .field("current", &self.current)
            .field("testnet", &self.testnet)
            .finish_non_exhaustive()
    }
}

impl AccountApi {
    pub(crate) fn new(client: Arc<ApiClient>, testnet: bool) -> Self {
        let current = CurrentAccount::default();
        current.update(json!({
            "networkStatus": if client.is_socket_connected() { "connected" } else { "disconnected" },
            "network": client.network().as_str(),
        }));
        let api = Self {
            client,
            current,
            auth_projection: Arc::new(AsyncMutex::new(AuthProjectionState::default())),
            subscription_projection: Arc::new(Mutex::new(SubscriptionProjectionState::default())),
            listener_lifecycle: Arc::new(AccountListenerLifecycle::default()),
            testnet,
        };
        api.listen();
        api
    }

    #[must_use]
    pub fn current_account(&self) -> CurrentAccount {
        self.current.clone()
    }
}

fn data(value: &Value) -> &Value {
    value.get("data").unwrap_or(value)
}
