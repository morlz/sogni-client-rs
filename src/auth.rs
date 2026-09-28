use std::{
    fmt,
    future::Future,
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
    identity: Option<String>,
}

/// Internal ownership of asynchronous work, independent of token revisions.
#[derive(Clone)]
pub(crate) struct RequestSession {
    updates: watch::Receiver<u64>,
    id: u64,
}

struct RequestOperation {
    owner: watch::Receiver<u64>,
    session: u64,
    rejected_at: RwLock<Option<u64>>,
}

tokio::task_local! {
    // Nested guards share only their active future's rejection receipt. Other
    // concurrent requests still stop immediately when these credentials end.
    static REQUEST_OPERATION: Arc<RequestOperation>;
}

impl RequestSession {
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn check(&self) -> Result<()> {
        if *self.updates.borrow() != self.id {
            Err(Error::InvalidInput(
                "account session changed; submit this request again".into(),
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) async fn changed(&mut self) {
        while self.check().is_ok() {
            if self.updates.changed().await.is_err() {
                break;
            }
        }
    }

    pub(crate) fn run<T>(
        &self,
        future: impl Future<Output = Result<T>>,
    ) -> impl Future<Output = Result<T>> {
        let future = Box::pin(future);
        async move {
            self.check()?;
            let operation = REQUEST_OPERATION
                .try_with(Arc::clone)
                .ok()
                .filter(|operation| {
                    operation.owner.same_channel(&self.updates)
                        && (operation.session == self.id
                            || *operation.rejected_at.read() == Some(self.id))
                })
                .unwrap_or_else(|| {
                    Arc::new(RequestOperation {
                        owner: self.updates.clone(),
                        session: self.id,
                        rejected_at: RwLock::new(None),
                    })
                });
            let mut future = Box::pin(REQUEST_OPERATION.scope(operation.clone(), future));
            let mut session = self.clone();
            loop {
                tokio::select! {
                    biased;
                    () = session.changed() => {
                        if session.updates.has_changed().is_err() { return Err(Error::Closed); }
                        let current = *self.updates.borrow();
                        if operation.session == self.id && *operation.rejected_at.read() == Some(current) {
                            // Its own 401 body is still owned by the signed-out epoch.
                            // A subsequent login or logout must cancel it as usual.
                            session.id = current;
                        } else {
                            self.check()?;
                            return Err(Error::Closed);
                        }
                    },
                    result = &mut future => {
                        let owned_rejection = operation.session == self.id
                            && *operation.rejected_at.read() == Some(*self.updates.borrow());
                        let unauthorized = matches!(&result, Err(Error::Api(error)) if error.status == 401)
                            || matches!(&result, Err(Error::Chat(error)) if error.status == Some(401));
                        if !owned_rejection || !unauthorized { self.check()?; }
                        return result;
                    }
                }
            }
        }
    }

    pub(crate) fn record_matching_rejection(&self, signed_out: &Self) {
        let _ = REQUEST_OPERATION.try_with(|operation| {
            if operation.owner.same_channel(&self.updates) && operation.session == self.id {
                *operation.rejected_at.write() = Some(signed_out.id);
            }
        });
    }
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
    refresh_lock: RwLock<Arc<Mutex<()>>>,
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
                refresh_lock: RwLock::new(Arc::new(Mutex::new(()))),
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

    pub(crate) fn request_session(&self) -> RequestSession {
        let updates = self.subscribe_session();
        let id = *updates.borrow();
        RequestSession { updates, id }
    }

    pub(crate) fn version(&self) -> AuthVersion {
        self.inner.state.read().version
    }

    fn replace(&self, credentials: Credentials, authenticated: bool, identity: Option<String>) {
        let mut state = self.inner.state.write();
        let unchanged = match (&state.credentials, &credentials) {
            (Credentials::ApiKey(old), Credentials::ApiKey(new)) => old == new,
            (Credentials::Cookies, Credentials::Cookies) => {
                identity.is_none() || state.identity.is_none() || identity == state.identity
            }
            (Credentials::Tokens { token: old, .. }, Credentials::Tokens { token: new, .. }) => {
                match (&state.identity, &identity) {
                    (Some(old), Some(new)) => old == new,
                    _ => old == new,
                }
            }
            _ => false,
        };
        if !unchanged && !matches!(credentials, Credentials::Cookies) {
            self.inner.cookies.clear();
        }
        state.credentials = credentials;
        state.identity = identity.or_else(|| unchanged.then(|| state.identity.clone()).flatten());
        if !unchanged {
            state.version.session = state.version.session.wrapping_add(1);
            *self.inner.refresh_lock.write() = Arc::new(Mutex::new(()));
            self.inner.sessions.send_replace(state.version.session);
        }
        state.version.revision = state.version.revision.wrapping_add(1);
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
            None,
        );
        Ok(())
    }

    pub(crate) fn authenticate_cookies(&self) -> Result<()> {
        if self.inner.kind != AuthKind::Cookies {
            return Err(Error::InvalidInput(
                "cookie authentication was not configured".into(),
            ));
        }
        self.replace(Credentials::Cookies, true, None);
        Ok(())
    }

    pub(crate) fn set_cookie_identity(&self, identity: &str) {
        if self.kind() != AuthKind::Cookies {
            return;
        }
        let identity = identity.to_lowercase();
        let mut state = self.inner.state.write();
        if state.identity.as_ref().is_some_and(|old| old != &identity) {
            state.version.session = state.version.session.wrapping_add(1);
            state.version.revision = state.version.revision.wrapping_add(1);
            self.inner.sessions.send_replace(state.version.session);
            self.inner
                .updates
                .send_replace(matches!(state.credentials, Credentials::Cookies));
        }
        state.identity = Some(identity);
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
        let identity = jwt_identity(&token);
        self.replace(
            Credentials::Tokens {
                token,
                token_expires_at: token_exp,
                refresh_token,
                refresh_expires_at: refresh_exp,
            },
            refresh_exp > unix_time(),
            identity,
        );
        if token_exp <= unix_time() {
            self.renew_token().await?;
        }
        Ok(())
    }

    pub(crate) async fn headers(&self) -> Result<(AuthVersion, HeaderMap)> {
        let (session, needs_refresh) = {
            let state = self.inner.state.read();
            let needs_refresh = matches!(&state.credentials,
                Credentials::Tokens { token_expires_at, .. } if *token_expires_at <= unix_time());
            (state.version.session, needs_refresh)
        };
        if needs_refresh {
            self.renew_token().await?;
        }
        let state = self.inner.state.read();
        // A request waiting for another refresh still belongs to its original
        // account. Ordinary token revisions within that session remain valid.
        if state.version.session != session {
            return Err(Error::InvalidInput(
                "account session changed while preparing request headers".into(),
            ));
        }
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
        self.replace(Credentials::Empty, false, None);
    }

    pub(crate) fn clear_if_version(&self, expected: AuthVersion) -> Option<RequestSession> {
        let mut state = self.inner.state.write();
        if state.version != expected {
            return None;
        }
        self.inner.cookies.clear();
        state.credentials = Credentials::Empty;
        state.identity = None;
        state.version.session = state.version.session.wrapping_add(1);
        state.version.revision = state.version.revision.wrapping_add(1);
        self.inner.sessions.send_replace(state.version.session);
        self.inner.updates.send_replace(false);
        Some(self.request_session())
    }

    async fn renew_token(&self) -> Result<String> {
        let session = self.request_session();
        let refresh_lock = self.inner.refresh_lock.read().clone();
        let _guard = session.run(async { Ok(refresh_lock.lock().await) }).await?;
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
        let response = session
            .run(async {
                Ok(self
                    .inner
                    .http
                    .post(url)
                    .json(&json!({"refreshToken": refresh_token.as_str()}))
                    .send()
                    .await?)
            })
            .await?;
        let status = response.status();
        let text = session.run(async { Ok(response.text().await?) }).await?;
        let payload: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
            json!({"status": "error", "message": status.canonical_reason().unwrap_or("Token refresh failed"), "errorCode": status.as_u16()})
        });
        if !status.is_success() {
            if let Some(signed_out) = self.clear_if_version(version) {
                if status == reqwest::StatusCode::UNAUTHORIZED {
                    session.record_matching_rejection(&signed_out);
                }
            }
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
        let next_identity = jwt_identity(token);
        let identity_changed = state
            .identity
            .as_ref()
            .zip(next_identity.as_ref())
            .is_some_and(|(previous, next)| previous != next);
        if let Some(identity) = next_identity {
            state.identity = Some(identity);
        }
        state.credentials = Credentials::Tokens {
            token: Zeroizing::new(token.to_owned()),
            token_expires_at: token_exp,
            refresh_token: Zeroizing::new(next_refresh.to_owned()),
            refresh_expires_at: refresh_exp,
        };
        if identity_changed {
            state.version.session = state.version.session.wrapping_add(1);
            *self.inner.refresh_lock.write() = Arc::new(Mutex::new(()));
            self.inner.sessions.send_replace(state.version.session);
        }
        state.version.revision = state.version.revision.wrapping_add(1);
        self.inner.updates.send_replace(true);
        Ok(token.to_owned())
    }
}

fn jwt_exp(token: &str) -> Result<f64> {
    let value = jwt_payload(token)?;
    value
        .get("exp")
        .and_then(Value::as_f64)
        .or_else(|| value.get("exp").and_then(Value::as_i64).map(|v| v as f64))
        .ok_or_else(|| Error::InvalidInput("JWT payload has no numeric exp".into()))
}

fn jwt_identity(token: &str) -> Option<String> {
    let value = jwt_payload(token).ok()?;
    value
        .get("addr")
        .or_else(|| value.get("walletAddress"))
        .or_else(|| value.get("wallet_address"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase)
}

fn jwt_payload(token: &str) -> Result<Value> {
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
    serde_json::from_slice(&decoded).map_err(|_| Error::InvalidInput("invalid JWT payload".into()))
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

    #[tokio::test]
    async fn refreshed_wallet_identity_preserves_only_same_wallet_requests() {
        use crate::transport::{HttpClients, RestClient};
        use axum::{
            Json, Router,
            http::HeaderMap,
            routing::{get, post},
        };
        use std::{
            sync::atomic::{AtomicUsize, Ordering},
            time::Duration,
        };

        fn token(address: &str, serial: u8) -> String {
            format!(
                "e30.{}.fixture",
                URL_SAFE.encode(
                    json!({"addr":address,"serial":serial,"exp":4_102_444_800_u64}).to_string()
                )
            )
        }
        for next_wallet in ["0xaBcD", "0xDifferent"] {
            let next_token = token(next_wallet, 2);
            let response_token = next_token.clone();
            let requests = Arc::new(AtomicUsize::new(0));
            let counted = requests.clone();
            let expected = next_token.clone();
            let router = Router::new()
                .route(
                    "/v1/account/refresh-token",
                    post(move || {
                        let token = response_token.clone();
                        async move { Json(json!({"data":{"token":token,"refreshToken":token}})) }
                    }),
                )
                .route(
                    "/use",
                    get(move |headers: HeaderMap| {
                        let counted = counted.clone();
                        let expected = expected.clone();
                        async move {
                            assert_eq!(headers.get("authorization").unwrap(), expected.as_str());
                            counted.fetch_add(1, Ordering::SeqCst);
                            Json(json!({"data":"current"}))
                        }
                    }),
                );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint: Url = format!("http://{}/", listener.local_addr().unwrap())
                .parse()
                .unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let http = HttpClients::build(Duration::from_secs(5)).unwrap();
            let auth = AuthManager::new(
                AuthKind::Token,
                endpoint.clone(),
                http.authenticated(),
                http.cookies(),
            );
            auth.authenticate_tokens(token("0xABCD", 1), token("refresh", 1))
                .await
                .unwrap();
            let owner = auth.request_session();
            if let Credentials::Tokens {
                token_expires_at, ..
            } = &mut auth.inner.state.write().credentials
            {
                *token_expires_at = 1.0;
            }
            let rest = RestClient::new(endpoint, auth.clone(), http, Duration::from_secs(5));
            let result = rest.get("/use", None).await;
            if next_wallet == "0xaBcD" {
                assert!(result.is_ok());
                owner.check().unwrap();
                assert_eq!(requests.load(Ordering::SeqCst), 1);
            } else {
                assert!(matches!(result, Err(Error::InvalidInput(_))));
                assert!(owner.check().is_err());
                assert_eq!(
                    requests.load(Ordering::SeqCst),
                    0,
                    "an A request must not send B credentials"
                );
                assert_eq!(rest.get("/use", None).await.unwrap()["data"], "current");
            }
            assert_eq!(
                auth.inner.state.read().identity.as_deref(),
                Some(next_wallet.to_lowercase().as_str())
            );
            server.abort();
        }
    }
}
