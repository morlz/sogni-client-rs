use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use url::Url;

use crate::{Error, SogniClient};

const VALID_TOKEN: &str = "eyJhbGciOiJub25lIn0.eyJleHAiOjQxMDI0NDQ4MDB9.signature";
const EXPIRED_TOKEN: &str = "eyJhbGciOiJub25lIn0.eyJleHAiOjF9.signature";
const WALLET_ADDRESS: &str = "0x930fbddbfa47bf6263330d551414750c2688be5a";

#[derive(Clone, Default)]
struct FixtureState {
    me_calls: Arc<AtomicUsize>,
    me_failures_remaining: Arc<AtomicUsize>,
}

impl FixtureState {
    fn fail_next_me(&self) {
        self.me_failures_remaining.store(1, Ordering::SeqCst);
    }
}

async fn spawn_account_fixture() -> (Url, FixtureState, JoinHandle<()>) {
    let state = FixtureState::default();
    let app = Router::new()
        .route("/v1/account/nonce", post(nonce))
        .route("/v1/account/login", post(login))
        .route("/v1/account/me", get(me))
        .route("/v4/account/balance", get(unauthorized))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind account fixture");
    let address = listener.local_addr().expect("account fixture address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve account fixture");
    });
    (
        Url::parse(&format!("http://{address}/")).expect("account fixture URL"),
        state,
        server,
    )
}

async fn nonce() -> Json<Value> {
    Json(json!({"data": {"nonce": "fixture-nonce"}}))
}

async fn login() -> Json<Value> {
    Json(json!({
        "data": {"token": VALID_TOKEN, "refreshToken": VALID_TOKEN}
    }))
}

async fn me(State(state): State<FixtureState>) -> (StatusCode, Json<Value>) {
    state.me_calls.fetch_add(1, Ordering::SeqCst);
    if state
        .me_failures_remaining
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            remaining.checked_sub(1)
        })
        .is_ok()
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"message": "transient account lookup failure"})),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "data": {
                "username": "fixture-user",
                "currentEmail": "fixture@example.com",
                "walletAddress": WALLET_ADDRESS,
            }
        })),
    )
}

async fn unauthorized() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"message": "expired", "errorCode": 401})),
    )
}

async fn fixture_client(endpoint: Url) -> SogniClient {
    SogniClient::builder()
        .app_id("account-auth-state-test")
        .rest_endpoint(endpoint)
        .disable_socket(true)
        .request_timeout(Duration::from_secs(2))
        .build()
        .await
        .expect("build fixture client")
}

#[tokio::test]
async fn configured_tokens_await_one_current_account_hydration() {
    let (endpoint, fixture, server) = spawn_account_fixture().await;
    let client = SogniClient::builder()
        .app_id("account-auth-state-test")
        .rest_endpoint(endpoint)
        .tokens(VALID_TOKEN, VALID_TOKEN)
        .disable_socket(true)
        .request_timeout(Duration::from_secs(2))
        .build()
        .await
        .expect("build authenticated fixture client");

    assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        client.current_account().wallet_address().as_deref(),
        Some(WALLET_ADDRESS)
    );
    client.close().await.expect("close client");
    server.abort();
}

async fn wait_until_cleared(client: &SogniClient) {
    let current = client.current_account();
    let mut updates = current.subscribe();
    tokio::time::timeout(Duration::from_secs(2), async {
        while current.is_authenticated() {
            updates.recv().await.expect("account update channel");
        }
    })
    .await
    .expect("account projection should clear");
}

async fn wait_for_listener_shutdown(probes: &[std::sync::Weak<()>]) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while probes.iter().any(|probe| probe.upgrade().is_some()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("account listeners should terminate");
}

async fn assert_failed_hydration_was_rolled_back(client: &SogniClient) {
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert!(!client.is_authenticated());
    assert!(client.auth_backup().expect("auth backup state").is_none());
    let current = client.current_account();
    assert_eq!(current.username(), None);
    assert_eq!(current.email(), None);
    assert_eq!(current.wallet_address(), None);
    assert_eq!(current.subscription(), None);
    let projection = client.account.auth_projection.lock().await;
    assert!(!projection.hydrated);
    assert!(!projection.skip_next_authenticated_update);
}

#[tokio::test]
async fn explicit_close_joins_both_account_listeners() {
    let (endpoint, _fixture, server) = spawn_account_fixture().await;
    let client = fixture_client(endpoint).await;
    let probes = client.account.listener_task_probes();
    assert_eq!(probes.len(), 2);

    client.close().await.expect("close client");
    wait_for_listener_shutdown(&probes).await;

    server.abort();
}

#[tokio::test]
async fn final_handle_drop_releases_account_listener_state() {
    let (endpoint, _fixture, server) = spawn_account_fixture().await;
    let client = fixture_client(endpoint).await;
    let probes = client.account.listener_task_probes();
    let api_client = client.account.client_retention_probe();
    let retained_projects = client.projects.clone();
    let current = client.current_account();
    let current_state = current.state_retention_probe();
    drop(current);

    drop(client);
    wait_for_listener_shutdown(&probes).await;

    assert!(api_client.upgrade().is_some());
    assert!(current_state.upgrade().is_none());
    drop(retained_projects);
    assert!(api_client.upgrade().is_none());
    server.abort();
}

#[cfg(feature = "wallet")]
#[tokio::test]
async fn login_awaits_one_current_account_hydration() {
    let (endpoint, fixture, server) = spawn_account_fixture().await;
    let client = fixture_client(endpoint).await;

    client
        .account
        .login("TestUser", "correct horse battery staple")
        .await
        .expect("login");

    assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 1);
    let current = client.current_account();
    assert_eq!(current.username().as_deref(), Some("fixture-user"));
    assert_eq!(current.email().as_deref(), Some("fixture@example.com"));
    assert_eq!(current.wallet_address().as_deref(), Some(WALLET_ADDRESS));
    client.close().await.expect("close client");
    server.abort();
}

#[cfg(feature = "wallet")]
#[tokio::test]
async fn failed_login_hydration_rolls_back_auth_and_allows_retry() {
    let (endpoint, fixture, server) = spawn_account_fixture().await;
    fixture.fail_next_me();
    let client = fixture_client(endpoint).await;
    client.current_account().update(json!({
        "username": "stale-user",
        "walletAddress": "0xstale",
        "subscription": {"active": true},
    }));

    let error = client
        .account
        .login("TestUser", "correct horse battery staple")
        .await
        .expect_err("first account hydration fails");
    assert!(matches!(error, Error::Api(ref error) if error.status == 503));
    assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 1);
    assert_failed_hydration_was_rolled_back(&client).await;

    client
        .account
        .login("TestUser", "correct horse battery staple")
        .await
        .expect("retry login");
    assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 2);
    assert!(client.is_authenticated());
    assert_eq!(
        client.current_account().username().as_deref(),
        Some("fixture-user")
    );
    client.close().await.expect("close client");
    server.abort();
}

#[tokio::test]
async fn failed_set_tokens_hydration_rolls_back_auth_and_allows_retry() {
    let (endpoint, fixture, server) = spawn_account_fixture().await;
    fixture.fail_next_me();
    let client = fixture_client(endpoint).await;
    client.current_account().update(json!({
        "username": "stale-user",
        "walletAddress": "0xstale",
        "subscription": {"active": true},
    }));

    let error = client
        .set_tokens(VALID_TOKEN, VALID_TOKEN)
        .await
        .expect_err("first account hydration fails");
    assert!(matches!(error, Error::Api(ref error) if error.status == 503));
    assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 1);
    assert_failed_hydration_was_rolled_back(&client).await;

    client
        .set_tokens(VALID_TOKEN, VALID_TOKEN)
        .await
        .expect("retry tokens");
    assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 2);
    assert!(client.is_authenticated());
    assert_eq!(
        client.current_account().username().as_deref(),
        Some("fixture-user")
    );
    client.close().await.expect("close client");
    server.abort();
}

#[tokio::test]
async fn unauthorized_response_clears_current_account_projection() {
    let (endpoint, fixture, server) = spawn_account_fixture().await;
    let client = fixture_client(endpoint).await;
    client
        .set_tokens(VALID_TOKEN, VALID_TOKEN)
        .await
        .expect("set valid tokens");
    let current = client.current_account();
    current.update(json!({"subscription": {"active": true}}));

    let error = client
        .account
        .account_balance()
        .await
        .expect_err("fixture rejects balance request");
    assert!(matches!(error, Error::Api(ref error) if error.status == 401));
    wait_until_cleared(&client).await;

    assert!(!client.is_authenticated());
    assert_eq!(client.current_account().username(), None);
    assert_eq!(client.current_account().subscription(), None);
    assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 1);
    client.close().await.expect("close client");
    server.abort();
}

#[tokio::test]
async fn expired_refresh_token_clears_current_account_projection() {
    let (endpoint, fixture, server) = spawn_account_fixture().await;
    let client = fixture_client(endpoint).await;
    client
        .set_tokens(VALID_TOKEN, VALID_TOKEN)
        .await
        .expect("set valid tokens");

    let error = client
        .set_tokens(EXPIRED_TOKEN, EXPIRED_TOKEN)
        .await
        .expect_err("expired refresh token is rejected");
    assert!(
        matches!(error, Error::InvalidInput(ref message) if message == "refresh token expired")
    );
    wait_until_cleared(&client).await;

    assert!(!client.is_authenticated());
    assert_eq!(client.current_account().wallet_address(), None);
    assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 1);
    client.close().await.expect("close client");
    server.abort();
}
