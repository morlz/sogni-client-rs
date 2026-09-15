use std::{
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use parking_lot::RwLock;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};
use url::Url;
use zeroize::Zeroizing;

use crate::{ApiError, Error, Result, transport::ClearableCookieStore};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuthKind {
    #[default]
    Token,
    Cookies,
    ApiKey,
}

#[derive(Clone)]
pub enum AuthBackup {
    ApiKey(String),
    Tokens {
        token: String,
        refresh_token: String,
    },
}

impl fmt::Debug for AuthBackup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey(_) => f.write_str("AuthBackup::ApiKey([REDACTED])"),
            Self::Tokens { .. } => f.write_str("AuthBackup::Tokens([REDACTED])"),
        }
    }
}

#[derive(Default)]
enum Credentials {
    #[default]
    Empty,
    ApiKey(Zeroizing<String>),
    Tokens {
        token: Zeroizing<String>,
        token_expires_at: f64,
        refresh_token: Zeroizing<String>,
        refresh_expires_at: f64,
    },
    Cookies,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AuthVersion {
    pub(crate) session: u64,
    revision: u64,
}

#[derive(Default)]
struct AuthState {
    credentials: Credentials,
    version: AuthVersion,
}

#[derive(Clone)]
pub(crate) struct AuthManager {
    inner: Arc<AuthInner>,
}

struct AuthInner {
    kind: AuthKind,
    base_url: Url,
    http: reqwest::Client,
    cookies: Arc<ClearableCookieStore>,
    state: RwLock<AuthState>,
    refresh_lock: Mutex<()>,
    updates: watch::Sender<bool>,
    sessions: watch::Sender<u64>,
}

impl fmt::Debug for AuthManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthManager")
            .field("kind", &self.inner.kind)
            .field("is_authenticated", &self.is_authenticated())
            .finish_non_exhaustive()
    }
}

impl AuthManager {
    pub(crate) fn new(
        kind: AuthKind,
        base_url: Url,
        http: reqwest::Client,
        cookies: Arc<ClearableCookieStore>,
    ) -> Self {
        let (updates, _) = watch::channel(false);
        let (sessions, _) = watch::channel(0);
        Self {
            inner: Arc::new(AuthInner {
                kind,
                base_url,
                http,
                cookies,
                state: RwLock::new(AuthState::default()),
                refresh_lock: Mutex::new(()),
                updates,
                sessions,
            }),
        }
    }

    pub(crate) fn kind(&self) -> AuthKind {
        self.inner.kind
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.inner.updates.subscribe()
    }

    pub(crate) fn subscribe_session(&self) -> watch::Receiver<u64> {
        self.inner.sessions.subscribe()
    }

    pub(crate) fn version(&self) -> AuthVersion {
        self.inner.state.read().version
    }

    fn replace(&self, credentials: Credentials, authenticated: bool) {
        let mut state = self.inner.state.write();
        if !matches!(credentials, Credentials::Cookies) {
            self.inner.cookies.clear();
        }
        state.credentials = credentials;
        state.version.session = state.version.session.wrapping_add(1);
        state.version.revision = state.version.revision.wrapping_add(1);
        self.inner.sessions.send_replace(state.version.session);
        self.inner.updates.send_replace(authenticated);
    }

    pub(crate) fn is_authenticated(&self) -> bool {
        let now = unix_time();
        match &self.inner.state.read().credentials {
            Credentials::ApiKey(key) => !key.trim().is_empty(),
            Credentials::Tokens {
                refresh_token,
                refresh_expires_at,
                ..
            } => !refresh_token.is_empty() && *refresh_expires_at > now,
            Credentials::Cookies => true,
            Credentials::Empty => false,
        }
    }

    pub(crate) fn authenticate_api_key(&self, api_key: impl Into<String>) -> Result<()> {
        if self.inner.kind != AuthKind::ApiKey {
            return Err(Error::InvalidInput(
                "API keys require AuthKind::ApiKey".into(),
            ));
        }
        let api_key = Zeroizing::new(api_key.into());
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(Error::InvalidInput("api_key must be non-empty".into()));
        }
        self.replace(
            Credentials::ApiKey(Zeroizing::new(api_key.to_owned())),
            true,
        );
        Ok(())
    }

    pub(crate) fn authenticate_cookies(&self) -> Result<()> {
        if self.inner.kind != AuthKind::Cookies {
            return Err(Error::InvalidInput(
                "cookie authentication was not configured".into(),
            ));
        }
        self.replace(Credentials::Cookies, true);
        Ok(())
    }

    pub(crate) async fn authenticate_tokens(
        &self,
        token: impl Into<String>,
        refresh_token: impl Into<String>,
    ) -> Result<()> {
        if self.inner.kind != AuthKind::Token {
            return Err(Error::InvalidInput("tokens require AuthKind::Token".into()));
        }
        let token = Zeroizing::new(token.into());
        let refresh_token = Zeroizing::new(refresh_token.into());
        if token.is_empty() || refresh_token.is_empty() {
            return Err(Error::InvalidInput(
                "both token and refresh_token are required".into(),
            ));
        }
        let token_exp = jwt_exp(&token)?;
        let refresh_exp = jwt_exp(&refresh_token)?;
        self.replace(
            Credentials::Tokens {
                token,
                token_expires_at: token_exp,
                refresh_token,
                refresh_expires_at: refresh_exp,
            },
            refresh_exp > unix_time(),
        );
        if token_exp <= unix_time() {
            self.renew_token().await?;
        }
        Ok(())
    }

    pub(crate) async fn headers(&self) -> Result<(AuthVersion, HeaderMap)> {
        let needs_refresh = matches!(&self.inner.state.read().credentials,
            Credentials::Tokens { token_expires_at, .. } if *token_expires_at <= unix_time());
        if needs_refresh {
            self.renew_token().await?;
        }
        let state = self.inner.state.read();
        let mut headers = HeaderMap::new();
        match &state.credentials {
            Credentials::ApiKey(key) => {
                headers.insert(
                    HeaderName::from_static("api-key"),
                    secret_header(key.as_str())?,
                );
            }
            Credentials::Tokens { token, .. } => {
                headers.insert(AUTHORIZATION, secret_header(token)?);
            }
            _ => {}
        }
        Ok((state.version, headers))
    }

    pub(crate) fn socket_cookie(&self, url: &Url) -> Option<HeaderValue> {
        if self.kind() != AuthKind::Cookies {
            return None;
        }
        let mut url = url.clone();
        let scheme = if url.scheme() == "wss" {
            "https"
        } else {
            "http"
        };
        url.set_scheme(scheme).ok()?;
        self.inner.cookies.request_header(&url).1.map(|mut cookie| {
            cookie.set_sensitive(true);
            cookie
        })
    }

    pub(crate) fn backup(&self) -> Result<Option<AuthBackup>> {
        match &self.inner.state.read().credentials {
            Credentials::ApiKey(key) => Ok(Some(AuthBackup::ApiKey(key.to_string()))),
            Credentials::Tokens {
                token,
                refresh_token,
                ..
            } => Ok(Some(AuthBackup::Tokens {
                token: token.to_string(),
                refresh_token: refresh_token.to_string(),
            })),
            Credentials::Cookies => Err(Error::InvalidInput(
                "cookie authentication cannot be backed up".into(),
            )),
            Credentials::Empty => Ok(None),
        }
    }

    pub(crate) fn clear(&self) {
        self.replace(Credentials::Empty, false);
    }

    pub(crate) fn clear_if_version(&self, expected: AuthVersion) {
        let mut state = self.inner.state.write();
        if state.version != expected {
            return;
        }
        self.inner.cookies.clear();
        state.credentials = Credentials::Empty;
        state.version.session = state.version.session.wrapping_add(1);
        state.version.revision = state.version.revision.wrapping_add(1);
        self.inner.sessions.send_replace(state.version.session);
        self.inner.updates.send_replace(false);
    }

    async fn renew_token(&self) -> Result<String> {
        let _guard = self.inner.refresh_lock.lock().await;
        let (version, refresh_token) = {
            let guard = self.inner.state.read();
            let version = guard.version;
            match &guard.credentials {
                Credentials::Tokens {
                    token,
                    token_expires_at,
                    ..
                } if *token_expires_at > unix_time() => return Ok(token.to_string()),
                Credentials::Tokens {
                    refresh_token,
                    refresh_expires_at,
                    ..
                } if *refresh_expires_at > unix_time() => {
                    (version, Zeroizing::new(refresh_token.to_string()))
                }
                Credentials::Tokens { .. } => {
                    drop(guard);
                    self.clear_if_version(version);
                    return Err(Error::InvalidInput("refresh token expired".into()));
                }
                _ => return Err(Error::InvalidInput("no refresh token is configured".into())),
            }
        };
        let url = self.inner.base_url.join("/v1/account/refresh-token")?;
        let response = self
            .inner
            .http
            .post(url)
            .json(&json!({"refreshToken": refresh_token.as_str()}))
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        let payload: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
            json!({"status": "error", "message": status.canonical_reason().unwrap_or("Token refresh failed"), "errorCode": status.as_u16()})
        });
        if !status.is_success() {
            self.clear_if_version(version);
            return Err(ApiError::new(status.as_u16(), payload).into());
        }
        let data = payload
            .get("data")
            .and_then(Value::as_object)
            .ok_or_else(|| Error::Protocol("token refresh response did not include data".into()))?;
        let token = data.get("token").and_then(Value::as_str).ok_or_else(|| {
            Error::Protocol("token refresh response did not include token".into())
        })?;
        let next_refresh = data
            .get("refreshToken")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Protocol("token refresh response did not include refreshToken".into())
            })?;
        let token_exp = jwt_exp(token)?;
        let refresh_exp = jwt_exp(next_refresh)?;
        let mut state = self.inner.state.write();
        if state.version != version {
            return Err(Error::InvalidInput(
                "account session changed during token refresh".into(),
            ));
        }
        state.credentials = Credentials::Tokens {
            token: Zeroizing::new(token.to_owned()),
            token_expires_at: token_exp,
            refresh_token: Zeroizing::new(next_refresh.to_owned()),
            refresh_expires_at: refresh_exp,
        };
        state.version.revision = state.version.revision.wrapping_add(1);
        self.inner.updates.send_replace(true);
        Ok(token.to_owned())
    }
}

fn jwt_exp(token: &str) -> Result<f64> {
    let raw = token.strip_prefix("Bearer ").unwrap_or(token).trim();
    let payload = raw
        .split('.')
        .nth(1)
        .ok_or_else(|| Error::InvalidInput("invalid JWT".into()))?;
    let mut padded = payload.to_owned();
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    let decoded = URL_SAFE
        .decode(padded)
        .map_err(|_| Error::InvalidInput("invalid JWT".into()))?;
    let value: Value = serde_json::from_slice(&decoded)
        .map_err(|_| Error::InvalidInput("invalid JWT payload".into()))?;
    value
        .get("exp")
        .and_then(Value::as_f64)
        .or_else(|| value.get("exp").and_then(Value::as_i64).map(|v| v as f64))
        .ok_or_else(|| Error::InvalidInput("JWT payload has no numeric exp".into()))
}

fn unix_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64())
}

fn secret_header(value: &str) -> Result<HeaderValue> {
    let mut value = HeaderValue::from_str(value)
        .map_err(|_| Error::InvalidInput("credential contains invalid header bytes".into()))?;
    value.set_sensitive(true);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_cookies_respect_domain_path_and_secure() {
        let url = Url::parse("https://api.example.test/login").unwrap();
        let cookies = Arc::new(ClearableCookieStore::default());
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::SET_COOKIE,
            HeaderValue::from_static(
                "session=fixture; Domain=example.test; Path=/socket; Secure; HttpOnly",
            ),
        );
        let (generation, _) = cookies.request_header(&url);
        cookies.store_response(generation, &headers, &url);
        let auth = AuthManager::new(AuthKind::Cookies, url, reqwest::Client::new(), cookies);
        auth.authenticate_cookies().unwrap();
        let header = auth
            .socket_cookie(&Url::parse("wss://socket.example.test/socket").unwrap())
            .unwrap();
        assert_eq!(header, "session=fixture");
        assert!(header.is_sensitive());
        for url in [
            "ws://socket.example.test/socket",
            "wss://other.test/socket",
            "wss://socket.example.test/other",
        ] {
            assert!(auth.socket_cookie(&Url::parse(url).unwrap()).is_none());
        }
        auth.clear();
        assert!(
            auth.socket_cookie(&Url::parse("wss://socket.example.test/socket").unwrap())
                .is_none()
        );
    }
}
