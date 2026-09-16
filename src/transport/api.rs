use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

use reqwest::header::HeaderMap;
use serde::Serialize;
use serde_json::Value;

use super::{HttpClients, RestClient, socket::SocketTransport};
use crate::{
    AuthBackup, AuthKind, ClientConfig, Error, EventReceiver, Network, Result, WorkloadAttribution,
    auth::AuthManager, event::EventBus,
};

pub struct ApiClient {
    config: ClientConfig,
    auth: AuthManager,
    pub rest: RestClient,
    socket: Option<SocketTransport>,
    events: EventBus,
    closed: AtomicBool,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient")
            .field("app_id", &self.config.app_id)
            .field("network", &self.config.network)
            .field("socket_enabled", &self.socket.is_some())
            .field("is_authenticated", &self.is_authenticated())
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    pub(crate) fn new(config: ClientConfig, auth: AuthManager, http: HttpClients) -> Result<Self> {
        let events = EventBus::default();
        let rest = RestClient::new(
            config.rest_endpoint.clone(),
            auth.clone(),
            http.clone(),
            config.request_timeout,
        );
        let socket = (!config.disable_socket)
            .then(|| SocketTransport::new(&config, auth.clone(), http, events.clone()))
            .transpose()?;
        Ok(Self {
            config,
            auth,
            rest,
            socket,
            events,
            closed: AtomicBool::new(false),
        })
    }

    #[must_use]
    pub fn app_id(&self) -> &str {
        &self.config.app_id
    }

    #[must_use]
    pub fn app_source(&self) -> Option<&str> {
        self.config.app_source.as_deref()
    }

    #[must_use]
    pub fn network(&self) -> Network {
        self.socket
            .as_ref()
            .map_or(self.config.network, SocketTransport::network)
    }

    #[must_use]
    pub fn auth_kind(&self) -> AuthKind {
        self.auth.kind()
    }

    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.auth.is_authenticated()
    }

    #[must_use]
    pub fn is_socket_connected(&self) -> bool {
        self.socket
            .as_ref()
            .is_some_and(SocketTransport::is_connected)
    }

    #[must_use]
    pub fn is_socket_authenticated(&self) -> bool {
        self.socket
            .as_ref()
            .is_some_and(SocketTransport::is_authenticated)
    }

    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.events.subscribe()
    }

    pub(crate) fn subscribe_auth(&self) -> tokio::sync::watch::Receiver<bool> {
        self.auth.subscribe()
    }

    pub(crate) fn subscribe_scoped(
        &self,
    ) -> tokio::sync::broadcast::Receiver<crate::event::ScopedEvent> {
        self.events.subscribe_scoped()
    }

    pub(crate) fn auth_session(&self) -> u64 {
        self.auth.version().session
    }

    pub async fn start(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        if let Some(socket) = &self.socket {
            if self.auth.is_authenticated() {
                socket.start().await?;
            }
        }
        Ok(())
    }

    pub async fn send_socket<T: Serialize>(&self, message_type: &str, data: &T) -> Result<()> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::InvalidInput("this client was created with disable_socket=true".into())
        })?;
        socket
            .send(message_type, &serde_json::to_value(data)?)
            .await
    }

    pub async fn socket_get(&self, path: &str, query: Option<&Value>) -> Result<Value> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::InvalidInput("this client was created with disable_socket=true".into())
        })?;
        socket.get(path, query).await
    }

    pub(crate) async fn send_socket_in_session<T: Serialize>(
        &self,
        message_type: &str,
        data: &T,
        session: u64,
    ) -> Result<()> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::InvalidInput("this client was created with disable_socket=true".into())
        })?;
        socket
            .send_in_session(message_type, &serde_json::to_value(data)?, session)
            .await
    }

    pub async fn switch_network(&self, network: Network) -> Result<Network> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::InvalidInput("network switching requires WebSocket transport".into())
        })?;
        socket.switch_network(network).await
    }

    pub async fn set_socket_event_subscriptions(
        &self,
        subscriptions: BTreeMap<String, bool>,
    ) -> Result<()> {
        let socket = self.socket.as_ref().ok_or_else(|| {
            Error::InvalidInput("socket event subscriptions require WebSocket transport".into())
        })?;
        socket.set_subscriptions(subscriptions).await
    }

    pub(crate) async fn set_tokens(&self, token: String, refresh_token: String) -> Result<()> {
        self.auth.authenticate_tokens(token, refresh_token).await
    }

    pub(crate) fn authenticate_cookies(&self) -> Result<()> {
        self.auth.authenticate_cookies()
    }

    pub(crate) fn auth_backup(&self) -> Result<Option<AuthBackup>> {
        self.auth.backup()
    }

    pub(crate) fn clear_auth(&self) {
        self.auth.clear();
    }

    pub(crate) fn resolve_workload_attribution(
        &self,
        override_value: Option<&WorkloadAttribution>,
        fallback_operation_id: Option<&str>,
    ) -> Option<WorkloadAttribution> {
        self.config
            .attribution
            .resolve_workload(override_value, fallback_operation_id)
    }

    pub(crate) fn attribution_headers(
        &self,
        app_source: Option<&str>,
        workload: Option<&WorkloadAttribution>,
    ) -> Result<HeaderMap> {
        let values = self.config.attribution.headers(app_source, workload);
        let mut headers = HeaderMap::new();
        for (name, value) in values {
            let name =
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
                    Error::InvalidInput(format!("invalid attribution header: {error}"))
                })?;
            let value = reqwest::header::HeaderValue::from_str(&value).map_err(|error| {
                Error::InvalidInput(format!("invalid attribution value: {error}"))
            })?;
            headers.insert(name, value);
        }
        Ok(headers)
    }

    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if let Some(socket) = &self.socket {
            socket.close().await?;
        }
        self.auth.clear();
        Ok(())
    }

    pub fn abort(&self) {
        self.closed.store(true, Ordering::Release);
        if let Some(socket) = &self.socket {
            socket.abort();
        }
        self.auth.clear();
    }
}

impl Drop for ApiClient {
    fn drop(&mut self) {
        if let Some(socket) = &self.socket {
            socket.cancel();
        }
    }
}
