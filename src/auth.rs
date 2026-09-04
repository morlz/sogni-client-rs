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

#[derive(Clone)]
pub(crate) struct AuthManager {
    inner: Arc<AuthInner>,
}

struct AuthInner {
    kind: AuthKind,
    base_url: Url,
    http: reqwest::Client,
    cookies: Arc<ClearableCookieStore>,
    credentials: RwLock<Credentials>,
    refresh_lock: Mutex<()>,
    updates: watch::Sender<bool>,
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
        Self {
            inner: Arc::new(AuthInner {
                kind,
                base_url,
                http,
                cookies,
                credentials: RwLock::new(Credentials::Empty),
                refresh_lock: Mutex::new(()),
                updates,
            }),
        }
    }

    pub(crate) fn kind(&self) -> AuthKind {
        self.inner.kind
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.inner.updates.subscribe()
    }

    pub(crate) fn is_authenticated(&self) -> bool {
        let now = unix_time();
        match &*self.inner.credentials.read() {
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
        *self.inner.credentials.write() = Credentials::ApiKey(Zeroizing::new(api_key.to_owned()));
        self.inner.updates.send_replace(true);
        Ok(())
    }

    pub(crate) fn authenticate_cookies(&self) -> Result<()> {
        if self.inner.kind != AuthKind::Cookies {
            return Err(Error::InvalidInput(
                "cookie authentication was not configured".into(),
            ));
        }
        *self.inner.credentials.write() = Credentials::Cookies;
        self.inner.updates.send_replace(true);
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
        *self.inner.credentials.write() = Credentials::Tokens {
            token,
            token_expires_at: token_exp,
            refresh_token,
            refresh_expires_at: refresh_exp,
        };
        self.inner.updates.send_replace(refresh_exp > unix_time());
        if token_exp <= unix_time() {
            self.renew_token().await?;
        }
        Ok(())
    }

    pub(crate) async fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        match self.inner.kind {
            AuthKind::ApiKey => {
                if let Credentials::ApiKey(key) = &*self.inner.credentials.read() {
                    headers.insert(
                        HeaderName::from_static("api-key"),
                        secret_header(key.as_str())?,
                    );
                }
            }
            AuthKind::Token => {
                let token = self.current_token().await?;
                if let Some(token) = token {
                    headers.insert(AUTHORIZATION, secret_header(&token)?);
                }
            }
            AuthKind::Cookies => {}
        }
        Ok(headers)
    }

    pub(crate) fn backup(&self) -> Result<Option<AuthBackup>> {
        match &*self.inner.credentials.read() {
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
        self.inner.cookies.clear();
        let had_credentials = !matches!(*self.inner.credentials.read(), Credentials::Empty);
        if had_credentials {
            *self.inner.credentials.write() = Credentials::Empty;
            self.inner.updates.send_replace(false);
        }
    }

    async fn current_token(&self) -> Result<Option<String>> {
        let current = {
            let guard = self.inner.credentials.read();
            match &*guard {
                Credentials::Tokens {
                    token,
                    token_expires_at,
                    ..
                } if *token_expires_at > unix_time() => return Ok(Some(token.to_string())),
                Credentials::Tokens { refresh_token, .. } => Some(refresh_token.to_string()),
                _ => None,
            }
        };
        if current.is_none() {
            return Ok(None);
        }
        self.renew_token().await.map(Some)
    }

    async fn renew_token(&self) -> Result<String> {
        let _guard = self.inner.refresh_lock.lock().await;
        let refresh_token = {
            let guard = self.inner.credentials.read();
            match &*guard {
                Credentials::Tokens {
                    token,
                    token_expires_at,
                    ..
                } if *token_expires_at > unix_time() => return Ok(token.to_string()),
                Credentials::Tokens {
                    refresh_token,
                    refresh_expires_at,
                    ..
                } if *refresh_expires_at > unix_time() => refresh_token.to_string(),
                Credentials::Tokens { .. } => {
                    drop(guard);
                    self.clear();
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
            .json(&json!({"refreshToken": refresh_token}))
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        let payload: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
            json!({"status": "error", "message": status.canonical_reason().unwrap_or("Token refresh failed"), "errorCode": status.as_u16()})
        });
        if !status.is_success() {
            self.clear();
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
        *self.inner.credentials.write() = Credentials::Tokens {
            token: Zeroizing::new(token.to_owned()),
            token_expires_at: token_exp,
            refresh_token: Zeroizing::new(next_refresh.to_owned()),
            refresh_expires_at: refresh_exp,
        };
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
    while padded.len() % 4 != 0 {
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
