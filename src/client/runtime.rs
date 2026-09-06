use std::{collections::BTreeMap, sync::Arc};

use super::config::{ClientBuilder, ClientConfig};
use crate::{
    AccountApi, AnnouncementsApi, AuthBackup, AuthKind, ChatApi, CreativeWorkflowsApi,
    CurrentAccount, Error, EventReceiver, ProjectsApi, ReplayApi, Result, StatsApi,
    auth::AuthManager,
    transport::{ApiClient, HttpClients},
};

#[derive(Clone)]
pub struct SogniClient {
    pub account: AccountApi,
    pub projects: ProjectsApi,
    pub stats: StatsApi,
    pub chat: ChatApi,
    pub workflows: CreativeWorkflowsApi,
    pub replay: ReplayApi,
    pub announcements: AnnouncementsApi,
    api_client: Arc<ApiClient>,
}

impl std::fmt::Debug for SogniClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SogniClient")
            .field("app_id", &self.api_client.app_id())
            .field("is_authenticated", &self.is_authenticated())
            .finish_non_exhaustive()
    }
}

impl SogniClient {
    #[must_use]
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    pub async fn create(mut config: ClientConfig) -> Result<Self> {
        if config.app_id.trim().is_empty() {
            return Err(Error::InvalidInput("app_id must be non-empty".into()));
        }
        if config.api_key.is_some() && config.auth_kind != AuthKind::ApiKey {
            return Err(Error::InvalidInput(
                "api_key requires AuthKind::ApiKey".into(),
            ));
        }
        let http = if config.strict_media_destinations {
            HttpClients::build_strict(config.request_timeout, config.proxy_url.as_deref())?
        } else if let Some(proxy) = config.proxy_url.as_deref() {
            HttpClients::build_with_proxy(config.request_timeout, proxy)?
        } else {
            HttpClients::build(config.request_timeout)?
        };
        let auth = AuthManager::new(
            config.auth_kind,
            config.rest_endpoint.clone(),
            http.authenticated(),
            http.cookies(),
        );
        if let Some(api_key) = config.api_key.take() {
            auth.authenticate_api_key(api_key)?;
        }
        match (config.token.take(), config.refresh_token.take()) {
            (Some(token), Some(refresh_token)) => {
                auth.authenticate_tokens(token, refresh_token).await?;
            }
            (None, None) => {}
            _ => {
                return Err(Error::InvalidInput(
                    "token and refresh_token must be configured together".into(),
                ));
            }
        }
        let testnet = config.testnet;
        let defer_socket_start = config.defer_socket_start;
        let api_client = Arc::new(ApiClient::new(config, auth, http)?);
        let projects = ProjectsApi::new(api_client.clone());
        let client = Self {
            account: AccountApi::new(api_client.clone(), testnet),
            stats: StatsApi::new(api_client.clone()),
            chat: ChatApi::new(api_client.clone(), projects.clone()),
            workflows: CreativeWorkflowsApi::new(api_client.clone()),
            replay: ReplayApi::new(api_client.clone()),
            announcements: AnnouncementsApi::new(api_client.clone()),
            projects,
            api_client,
        };
        if client.is_authenticated() {
            if client.api_client.auth_kind() == AuthKind::Token {
                client.account.hydrate_authenticated_projection().await?;
            }
            if !defer_socket_start {
                client.api_client.start().await?;
            }
        }
        Ok(client)
    }

    #[must_use]
    pub fn current_account(&self) -> CurrentAccount {
        self.account.current_account()
    }

    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.api_client.is_authenticated()
    }

    /// Whether the realtime transport currently has a connected socket.
    #[must_use]
    pub fn is_socket_connected(&self) -> bool {
        self.api_client.is_socket_connected()
    }

    /// Whether this connection received the server's authenticated event.
    #[must_use]
    pub fn is_socket_authenticated(&self) -> bool {
        self.api_client.is_socket_authenticated()
    }

    /// Stop this client's realtime task and queued sends without awaiting I/O.
    /// This closes every clone sharing the client; it does not cancel remote jobs.
    /// Reconcile any submission already sent before relinquishing its identity.
    pub fn abort(&self) {
        self.api_client.abort();
    }

    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.api_client.subscribe()
    }

    pub fn auth_backup(&self) -> Result<Option<AuthBackup>> {
        self.api_client.auth_backup()
    }

    pub async fn set_tokens(
        &self,
        token: impl Into<String>,
        refresh_token: impl Into<String>,
    ) -> Result<()> {
        self.account
            .set_tokens_and_hydrate(token.into(), refresh_token.into())
            .await?;
        self.api_client.start().await
    }

    pub async fn check_auth(&self) -> Result<bool> {
        if self.api_client.auth_kind() != AuthKind::Cookies {
            return Err(Error::InvalidInput(
                "check_auth is only valid for cookie authentication".into(),
            ));
        }
        match self.account.me().await {
            Ok(_) => {
                self.api_client.authenticate_cookies()?;
                self.api_client.start().await?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    pub async fn set_socket_event_subscriptions(
        &self,
        subscriptions: BTreeMap<String, bool>,
    ) -> Result<()> {
        self.api_client
            .set_socket_event_subscriptions(subscriptions)
            .await
    }

    pub async fn close(&self) -> Result<()> {
        self.account.stop_listeners().await;
        let result = self.api_client.close().await;
        self.account.clear_deauthenticated_projection().await;
        result
    }
}
