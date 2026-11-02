use crate::{ACCOUNT_RESTORABLE_ERROR_CODE, Error, LoginOptions, SogniClient};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const TOKEN: &str = "eyJhbGciOiJub25lIn0.eyJleHAiOjQxMDI0NDQ4MDB9.signature";
const ADDRESS: &str = "0x930fbddbfa47bf6263330d551414750c2688be5a";
#[derive(Default)]
struct FixtureState {
    deleted: AtomicBool,
    nonces: Mutex<Vec<Value>>,
    logins: Mutex<Vec<Value>>,
}
struct Fixture {
    client: SogniClient,
    state: Arc<FixtureState>,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start(deleted: bool) -> Self {
        let state = Arc::new(FixtureState::default());
        state.deleted.store(deleted, Ordering::SeqCst);
        let router = Router::new()
            .route("/v1/account/nonce", post(nonce))
            .route("/v1/account/login", post(login))
            .route(
                "/v1/account/me",
                get(|| async {
                    Json(json!({"data":{"username":"TestUser","walletAddress":ADDRESS}}))
                }),
            )
            .route(
                "/v4/account/balance",
                get(|| async { Json(json!({"data":{}})) }),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = SogniClient::builder()
            .app_source("sdk-test")
            .disable_socket(true)
            .rest_endpoint(format!("http://{address}").parse().unwrap())
            .socket_endpoint(format!("ws://{address}").parse().unwrap())
            .build()
            .await
            .unwrap();
        Self {
            client,
            state,
            server,
        }
    }
    async fn close(self) {
        self.client.close().await.unwrap();
        self.server.abort();
        let _ = self.server.await;
    }
}
async fn nonce(State(state): State<Arc<FixtureState>>, Json(body): Json<Value>) -> Json<Value> {
    state.nonces.lock().push(body);
    Json(json!({"data":{"nonce":"fixture-nonce"}}))
}
async fn login(
    State(state): State<Arc<FixtureState>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.logins.lock().push(body.clone());
    if state.deleted.load(Ordering::SeqCst)
        && body.get("restoreDeletedAccount") != Some(&json!(true))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"status":"error","errorCode":196,
            "message":"This account was deleted. Sign in at app.sogni.ai to restore it.","details":{"deletedAt":1761955200}})),
        );
    }
    state.deleted.store(false, Ordering::SeqCst);
    (
        StatusCode::OK,
        Json(json!({"data":{"token":TOKEN,"refreshToken":TOKEN,"username":"TestUser"}})),
    )
}

#[tokio::test]
async fn account_login_5_72_preserves_legacy_calls_and_omits_unset_restoration() {
    let fixture = Fixture::start(false).await;
    fixture
        .client
        .account
        .login("TestUser", "correct horse battery staple")
        .await
        .unwrap();
    fixture
        .client
        .account
        .login_with_settings(
            "TestUser",
            "correct horse battery staple",
            LoginOptions::default(),
        )
        .await
        .unwrap();
    fixture
        .client
        .account
        .login_with_options(
            "TestUser",
            "correct horse battery staple",
            true,
            Some("web-app"),
        )
        .await
        .unwrap();
    let logins = fixture.state.logins.lock().clone();
    assert_eq!(logins.len(), 3);
    assert_eq!(logins[0], logins[1]);
    for body in &logins {
        assert!(body.get("restoreDeletedAccount").is_none());
        assert_eq!(body["walletAddress"], ADDRESS);
    }
    assert_eq!(logins[0]["rememberMe"], false);
    assert_eq!(logins[0]["appSource"], "sdk-test");
    assert_eq!(logins[2]["rememberMe"], true);
    assert_eq!(logins[2]["appSource"], "web-app");
    assert_eq!(
        fixture.state.nonces.lock().as_slice(),
        vec![json!({"walletAddress":ADDRESS}); 3]
    );
    fixture.close().await;
}

#[tokio::test]
async fn account_login_5_72_requires_explicit_restore_and_repeats_session_settings() {
    let fixture = Fixture::start(true).await;
    let error = fixture
        .client
        .account
        .login_with_options(
            "TestUser",
            "correct horse battery staple",
            true,
            Some("web-app"),
        )
        .await
        .unwrap_err();
    let Error::Api(error) = error else {
        panic!("expected restorable API failure")
    };
    assert_eq!(error.status, 400);
    assert_eq!(error.error_code, ACCOUNT_RESTORABLE_ERROR_CODE);
    assert_eq!(error.details().unwrap()["deletedAt"], 1761955200_i64);
    assert_eq!(error.payload["details"]["deletedAt"], 1761955200_i64);
    assert!(!fixture.client.is_authenticated());
    assert_eq!(
        fixture.state.logins.lock().len(),
        1,
        "never retry or restore automatically"
    );
    let settings = LoginOptions {
        remember_me: true,
        app_source: Some("web-app".into()),
        restore_deleted_account: true,
    };
    fixture
        .client
        .account
        .login_with_settings("TestUser", "correct horse battery staple", settings)
        .await
        .unwrap();
    assert!(fixture.client.is_authenticated());
    let logins = fixture.state.logins.lock().clone();
    assert_eq!(logins.len(), 2);
    let mut expected = logins[0].clone();
    expected["restoreDeletedAccount"] = json!(true);
    assert_eq!(logins[1], expected);
    assert_eq!(fixture.state.nonces.lock().len(), 2);
    fixture.close().await;
}
