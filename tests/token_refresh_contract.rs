use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sogni_client::{AuthBackup, Error, SogniClient};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};

const WATCHDOG: Duration = Duration::from_secs(10);
const WALLET: &str = "0xAbCd";

#[derive(Clone, Default)]
struct Gate {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

impl Gate {
    async fn hold(&self) {
        self.started.notify_one();
        self.release.notified().await;
    }

    async fn wait(&self) {
        tokio::time::timeout(WATCHDOG, self.started.notified())
            .await
            .expect("fixture request must arrive");
    }
}

#[derive(Clone)]
struct Fixture {
    status: StatusCode,
    body: String,
    refresh_calls: Arc<AtomicUsize>,
    me_calls: Arc<AtomicUsize>,
    authorization: Arc<Mutex<Option<String>>>,
    refresh_gate: Option<Gate>,
    balance_gate: Option<Gate>,
}

impl Fixture {
    fn new(body: impl Into<String>) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.into(),
            refresh_calls: Arc::new(AtomicUsize::new(0)),
            me_calls: Arc::new(AtomicUsize::new(0)),
            authorization: Arc::new(Mutex::new(None)),
            refresh_gate: None,
            balance_gate: None,
        }
    }

    fn record_authorization(&self, headers: &HeaderMap) {
        *self.authorization.lock() = headers
            .get("authorization")
            .map(|value| value.to_str().unwrap().to_owned());
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn token(wallet: &str, expires_at: f64) -> String {
    format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(json!({"addr": wallet, "exp": expires_at}).to_string())
    )
}

async fn refresh(State(fixture): State<Fixture>, Json(body): Json<Value>) -> (StatusCode, String) {
    assert!(body["refreshToken"].as_str().is_some());
    fixture.refresh_calls.fetch_add(1, Ordering::SeqCst);
    if let Some(gate) = fixture.refresh_gate.as_ref() {
        gate.hold().await;
    }
    (fixture.status, fixture.body)
}

async fn me(State(fixture): State<Fixture>, headers: HeaderMap) -> Json<Value> {
    fixture.me_calls.fetch_add(1, Ordering::SeqCst);
    fixture.record_authorization(&headers);
    Json(json!({"data": {
        "username": "refresh-fixture",
        "walletAddress": WALLET,
        "email": "refresh@example.test"
    }}))
}

async fn balance(State(fixture): State<Fixture>, headers: HeaderMap) -> Json<Value> {
    fixture.record_authorization(&headers);
    if let Some(gate) = fixture.balance_gate.as_ref() {
        gate.hold().await;
    }
    Json(json!({"data": {"spark": "1"}}))
}

async fn client(fixture: Fixture) -> (SogniClient, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/account/refresh-token", post(refresh))
        .route("/v1/account/me", get(me))
        .route("/v4/account/balance", get(balance))
        .with_state(fixture);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = SogniClient::builder()
        .rest_endpoint(endpoint.parse().unwrap())
        .disable_socket(true)
        .request_timeout(WATCHDOG)
        .build()
        .await
        .unwrap();
    (client, server)
}

fn assert_backup(client: &SogniClient, expected_token: &str, expected_refresh: &str) {
    match client.auth_backup().unwrap().unwrap() {
        AuthBackup::Tokens {
            token,
            refresh_token,
        } => {
            assert_eq!(token, expected_token);
            assert_eq!(refresh_token, expected_refresh);
        }
        AuthBackup::ApiKey(_) => panic!("expected token authentication"),
    }
}

#[tokio::test]
async fn malformed_successful_refresh_signs_out_without_retrying() {
    let valid = token(WALLET, now() + 3600.0);
    let cases = [
        ("missing data", json!({"status": "ok"}).to_string()),
        ("array", json!([]).to_string()),
        ("null", json!(null).to_string()),
        ("wrong data type", json!({"data": []}).to_string()),
        ("missing tokens", json!({"data": {}}).to_string()),
        (
            "malformed tokens",
            json!({"data": {"token": "x", "refreshToken": "y"}}).to_string(),
        ),
        (
            "malformed refresh token",
            json!({"data": {"token": valid, "refreshToken": "y"}}).to_string(),
        ),
        (
            "missing refresh token",
            json!({"data": {"token": valid}}).to_string(),
        ),
        (
            "missing access token",
            json!({"data": {"refreshToken": valid}}).to_string(),
        ),
        (
            "non-string token",
            json!({"data": {"token": 1, "refreshToken": valid}}).to_string(),
        ),
        (
            "non-string refresh token",
            json!({"data": {"token": valid, "refreshToken": false}}).to_string(),
        ),
        (
            "empty token",
            json!({"data": {"token": "", "refreshToken": valid}}).to_string(),
        ),
        (
            "empty refresh token",
            json!({"data": {"token": valid, "refreshToken": ""}}).to_string(),
        ),
        ("unparseable JSON", "<html>fixture response</html>".into()),
        ("empty response", String::new()),
    ];
    for (label, body) in cases {
        let fixture = Fixture::new(body);
        let (client, server) = client(fixture.clone()).await;
        let error = client
            .set_tokens(
                token(WALLET, now() - 60.0),
                token("refresh", now() + 7200.0),
            )
            .await
            .expect_err(label);
        assert!(
            matches!(error, Error::Protocol(ref message) if message ==
                "token refresh response did not include a valid token pair (HTTP 200)"),
            "{label}: {error}"
        );
        assert!(!client.is_authenticated(), "{label}");
        assert!(client.auth_backup().unwrap().is_none(), "{label}");
        assert_eq!(client.current_account().wallet_address(), None, "{label}");
        assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 0, "{label}");

        client.account.account_balance().await.unwrap();
        assert_eq!(*fixture.authorization.lock(), None, "{label}");
        assert_eq!(fixture.refresh_calls.load(Ordering::SeqCst), 1, "{label}");
        client.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn valid_refresh_installs_both_tokens_before_hydration_and_requests() {
    let renewed = token(WALLET, now() + 3600.0);
    let next_refresh = token("refresh-next", now() + 7200.0);
    let fixture =
        Fixture::new(json!({"data": {"token": renewed, "refreshToken": next_refresh}}).to_string());
    let (client, server) = client(fixture.clone()).await;
    for wallet in [WALLET, "0xaBcD"] {
        client
            .set_tokens(
                token(wallet, now() - 60.0),
                token("refresh", now() + 7200.0),
            )
            .await
            .unwrap();
        assert!(client.is_authenticated());
        assert_backup(&client, &renewed, &next_refresh);
        assert_eq!(*fixture.authorization.lock(), Some(renewed.clone()));
        assert_eq!(
            client.current_account().wallet_address().as_deref(),
            Some(WALLET)
        );
    }
    client.account.account_balance().await.unwrap();
    assert_eq!(*fixture.authorization.lock(), Some(renewed));
    assert_eq!(fixture.refresh_calls.load(Ordering::SeqCst), 2);
    // Both installations hydrate before returning; the auth listener may also
    // refresh the same projection when it observes a valid renewal.
    assert!(fixture.me_calls.load(Ordering::SeqCst) >= 2);
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn invalid_refresh_clears_account_and_cancels_in_flight_session_work() {
    let refresh_gate = Gate::default();
    let balance_gate = Gate::default();
    let mut fixture =
        Fixture::new(json!({"data": {"token": "bad", "refreshToken": "bad"}}).to_string());
    fixture.refresh_gate = Some(refresh_gate.clone());
    fixture.balance_gate = Some(balance_gate.clone());
    let (client, server) = client(fixture.clone()).await;
    client
        .set_tokens(
            token(WALLET, now() + 3600.0),
            token("refresh", now() + 7200.0),
        )
        .await
        .unwrap();
    assert_eq!(
        client.current_account().username().as_deref(),
        Some("refresh-fixture")
    );
    let account = client.account.clone();
    let balance = tokio::spawn(async move { account.account_balance().await });
    balance_gate.wait().await;
    let renewing_client = client.clone();
    let renewal = tokio::spawn(async move {
        renewing_client
            .set_tokens(
                token(WALLET, now() - 60.0),
                token("refresh", now() + 7200.0),
            )
            .await
    });
    refresh_gate.wait().await;
    refresh_gate.release.notify_one();
    assert!(
        tokio::time::timeout(WATCHDOG, renewal)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    let error = tokio::time::timeout(WATCHDOG, balance)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(error, Error::InvalidInput(ref message) if message.contains("account session changed"))
    );
    assert!(!client.is_authenticated());
    assert!(client.auth_backup().unwrap().is_none());
    assert_eq!(client.current_account().wallet_address(), None);
    assert_eq!(client.current_account().username(), None);
    assert_eq!(client.current_account().email(), None);
    assert_eq!(fixture.refresh_calls.load(Ordering::SeqCst), 1);
    balance_gate.release.notify_one();
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn stale_malformed_refresh_cannot_clear_a_new_login_or_revive_a_closed_client() {
    for close in [false, true] {
        let gate = Gate::default();
        let mut fixture = Fixture::new(json!({"data": {}}).to_string());
        fixture.refresh_gate = Some(gate.clone());
        let (client, server) = client(fixture).await;
        let renewing_client = client.clone();
        let renewal = tokio::spawn(async move {
            renewing_client
                .set_tokens(
                    token(WALLET, now() - 60.0),
                    token("refresh", now() + 7200.0),
                )
                .await
        });
        gate.wait().await;
        let next_token = token("0xOther", now() + 3600.0);
        let next_refresh = token("refresh-other", now() + 7200.0);
        if close {
            client.close().await.unwrap();
        } else {
            client
                .set_tokens(next_token.clone(), next_refresh.clone())
                .await
                .unwrap();
        }
        gate.release.notify_one();
        let error = tokio::time::timeout(WATCHDOG, renewal)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        if close {
            assert!(matches!(error, Error::Closed));
            assert!(!client.is_authenticated());
            assert!(client.auth_backup().unwrap().is_none());
        } else {
            assert!(
                matches!(error, Error::InvalidInput(ref message) if message.contains("account session changed"))
            );
            assert!(client.is_authenticated());
            assert_backup(&client, &next_token, &next_refresh);
            client.close().await.unwrap();
        }
        server.abort();
    }
}
