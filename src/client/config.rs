use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};
use url::Url;

use super::runtime::SogniClient;
use crate::{Attribution, AuthKind, Result};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    #[default]
    Fast,
    Relaxed,
}

impl Network {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Relaxed => "relaxed",
        }
    }
}

#[derive(Clone)]
pub struct ClientConfig {
    /// Stable application-installation ID. Persist it across process restarts.
    /// May be empty only when `disable_socket` is true.
    pub app_id: String,
    pub app_source: Option<String>,
    pub attribution: Attribution,
    pub network: Network,
    pub auth_kind: AuthKind,
    pub api_key: Option<String>,
    pub token: Option<String>,
    pub refresh_token: Option<String>,
    pub rest_endpoint: Url,
    pub socket_endpoint: Url,
    pub socket_event_subscriptions: BTreeMap<String, bool>,
    /// REST-only mode: never open a socket. Socket generation/chat are unavailable.
    pub disable_socket: bool,
    /// Keep socket-hosted HTTP APIs available without starting realtime I/O at build.
    pub defer_socket_start: bool,
    pub testnet: bool,
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
    /// Explicit SOCKS5 route for HTTP, media and realtime transport.
    /// `socks5h` resolves service hostnames at the proxy; `socks5` resolves locally.
    pub proxy_url: Option<String>,
    /// Restrict provider media transfers to HTTPS and pinned public destinations.
    pub strict_media_destinations: bool,
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("app_id", &self.app_id)
            .field("app_source", &self.app_source)
            .field("attribution", &self.attribution)
            .field("network", &self.network)
            .field("auth_kind", &self.auth_kind)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("rest_endpoint", &self.rest_endpoint)
            .field("socket_endpoint", &self.socket_endpoint)
            .field(
                "socket_event_subscriptions",
                &self.socket_event_subscriptions,
            )
            .field("disable_socket", &self.disable_socket)
            .field("defer_socket_start", &self.defer_socket_start)
            .field("testnet", &self.testnet)
            .field("request_timeout", &self.request_timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("proxy_url", &self.proxy_url.as_ref().map(|_| "[REDACTED]"))
            .field("strict_media_destinations", &self.strict_media_destinations)
            .finish()
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_source: None,
            attribution: Attribution::default(),
            network: Network::Fast,
            auth_kind: AuthKind::Token,
            api_key: None,
            token: None,
            refresh_token: None,
            rest_endpoint: Url::parse("https://api.sogni.ai").expect("valid default REST URL"),
            socket_endpoint: Url::parse("wss://socket.sogni.ai")
                .expect("valid default WebSocket URL"),
            socket_event_subscriptions: BTreeMap::new(),
            disable_socket: false,
            defer_socket_start: false,
            testnet: false,
            request_timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(30),
            proxy_url: None,
            strict_media_destinations: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ClientBuilder {
    config: ClientConfig,
}

impl ClientBuilder {
    /// Set a persisted installation ID, unique among simultaneous account connections.
    #[must_use]
    pub fn app_id(mut self, value: impl Into<String>) -> Self {
        self.config.app_id = value.into();
        self
    }

    #[must_use]
    pub fn app_source(mut self, value: impl Into<String>) -> Self {
        self.config.app_source = Some(value.into());
        self
    }

    #[must_use]
    pub fn attribution(mut self, value: Attribution) -> Self {
        self.config.attribution = value;
        self
    }

    #[must_use]
    pub fn network(mut self, value: Network) -> Self {
        self.config.network = value;
        self
    }

    #[must_use]
    pub fn api_key(mut self, value: impl Into<String>) -> Self {
        self.config.auth_kind = AuthKind::ApiKey;
        self.config.api_key = Some(value.into());
        self
    }

    #[must_use]
    pub fn tokens(mut self, token: impl Into<String>, refresh_token: impl Into<String>) -> Self {
        self.config.auth_kind = AuthKind::Token;
        self.config.token = Some(token.into());
        self.config.refresh_token = Some(refresh_token.into());
        self
    }

    #[must_use]
    pub fn cookie_auth(mut self) -> Self {
        self.config.auth_kind = AuthKind::Cookies;
        self
    }

    #[must_use]
    pub fn rest_endpoint(mut self, value: Url) -> Self {
        self.config.rest_endpoint = value;
        self
    }

    #[must_use]
    pub fn socket_endpoint(mut self, value: Url) -> Self {
        self.config.socket_endpoint = value;
        self
    }

    /// Use only REST APIs; no app ID is required in this mode.
    #[must_use]
    pub fn disable_socket(mut self, value: bool) -> Self {
        self.config.disable_socket = value;
        self
    }

    /// Delay realtime connection until the first socket command. HTTP catalogue
    /// reads remain available and cannot replace an active app's socket session.
    #[must_use]
    pub fn defer_socket_start(mut self, value: bool) -> Self {
        self.config.defer_socket_start = value;
        self
    }

    #[must_use]
    pub fn testnet(mut self, value: bool) -> Self {
        self.config.testnet = value;
        self
    }

    #[must_use]
    pub fn request_timeout(mut self, value: Duration) -> Self {
        self.config.request_timeout = value;
        self
    }

    #[must_use]
    pub fn connect_timeout(mut self, value: Duration) -> Self {
        self.config.connect_timeout = value;
        self
    }

    #[must_use]
    pub fn proxy_url(mut self, value: impl Into<String>) -> Self {
        self.config.proxy_url = Some(value.into());
        self
    }

    #[must_use]
    pub fn strict_media_destinations(mut self, value: bool) -> Self {
        self.config.strict_media_destinations = value;
        self
    }

    #[must_use]
    pub fn socket_event_subscription(mut self, event: impl Into<String>, enabled: bool) -> Self {
        self.config
            .socket_event_subscriptions
            .insert(event.into(), enabled);
        self
    }

    pub async fn build(self) -> Result<SogniClient> {
        SogniClient::create(self.config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_redacts_credentials() {
        let config = ClientConfig {
            api_key: Some("api-key-secret".into()),
            token: Some("access-token-secret".into()),
            refresh_token: Some("refresh-token-secret".into()),
            ..ClientConfig::default()
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("api-key-secret"));
        assert!(!debug.contains("access-token-secret"));
        assert!(!debug.contains("refresh-token-secret"));
        assert!(!format!("{:?}", ClientBuilder { config }).contains("api-key-secret"));
    }
}
