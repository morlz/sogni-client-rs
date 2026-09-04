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
    http::{HeaderMap, HeaderValue, StatusCode, header::SET_COOKIE},
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use url::Url;

use crate::{Error, SogniClient};

const WALLET_ADDRESS: &str = "0x930fbddbfa47bf6263330d551414750c2688be5a";
const SESSION_COOKIE: &str = "fixture_session=active";

#[derive(Clone, Default)]
struct FixtureState {
    me_calls: Arc<AtomicUsize>,
    me_cookie_calls: Arc<AtomicUsize>,
    nonce_cookie_calls: Arc<AtomicUsize>,
}

async fn spawn_fixture() -> (Url, FixtureState, JoinHandle<()>) {
    let state = FixtureState::default();
    let app = Router::new()
        .route("/v1/account/nonce", post(nonce))
        .route("/v1/account/login", post(login))
        .route("/v1/account/me", get(me))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind cookie fixture");
    let address = listener.local_addr().expect("cookie fixture address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve cookie fixture");
    });
    (
        Url::parse(&format!("http://{address}/")).expect("cookie fixture URL"),
        state,
        server,
    )
}

async fn nonce(State(state): State<FixtureState>, headers: HeaderMap) -> Json<Value> {
    if has_session_cookie(&headers) {
        state.nonce_cookie_calls.fetch_add(1, Ordering::SeqCst);
    }
    Json(json!({"data": {"nonce": "fixture-nonce"}}))
}

async fn login() -> (HeaderMap, Json<Value>) {
    let mut headers = HeaderMap::new();
    headers.insert(
        SET_COOKIE,
        HeaderValue::from_static("fixture_session=active; Path=/; HttpOnly; SameSite=Lax"),
    );
    (headers, Json(json!({"data": {}})))
}

async fn me(State(state): State<FixtureState>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    let call = state.me_calls.fetch_add(1, Ordering::SeqCst);
    let has_cookie = has_session_cookie(&headers);
    if has_cookie {
        state.me_cookie_calls.fetch_add(1, Ordering::SeqCst);
    }
    if call == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"message": "transient account lookup failure"})),
        );
    }
    if !has_cookie {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "session cookie required"})),
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "data": {
                "username": "cookie-user",
                "currentEmail": "cookie@example.com",
                "walletAddress": WALLET_ADDRESS,
            }
        })),
    )
}

fn has_session_cookie(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(';').any(|part| part.trim() == SESSION_COOKIE))
}

#[tokio::test]
async fn failed_cookie_login_hydration_purges_cookie_before_retry() {
    let (endpoint, fixture, server) = spawn_fixture().await;
    let client = SogniClient::builder()
        .app_id("cookie-hydration-rollback-test")
        .rest_endpoint(endpoint)
        .cookie_auth()
        .disable_socket(true)
        .request_timeout(Duration::from_secs(2))
        .build()
        .await
        .expect("build cookie fixture client");

    let error = client
        .account
        .login("TestUser", "correct horse battery staple")
        .await
        .expect_err("first account hydration fails");
    assert!(matches!(error, Error::Api(ref error) if error.status == 503));
    assert!(!client.is_authenticated());
    assert_eq!(fixture.me_cookie_calls.load(Ordering::SeqCst), 1);

    client
        .account
        .login("TestUser", "correct horse battery staple")
        .await
        .expect("cookie login retry");
    assert_eq!(fixture.me_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.me_cookie_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.nonce_cookie_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        client.current_account().username().as_deref(),
        Some("cookie-user")
    );

    client.close().await.expect("close client");
    client
        .account
        .get_nonce(WALLET_ADDRESS)
        .await
        .expect("request after close remains cookie-free");
    assert_eq!(fixture.nonce_cookie_calls.load(Ordering::SeqCst), 0);
    server.abort();
}
