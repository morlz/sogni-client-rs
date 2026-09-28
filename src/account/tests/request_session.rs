use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use tokio::{sync::Notify, task::JoinHandle};

use crate::{AuthKind, Error, SogniClient};

const WATCHDOG: Duration = Duration::from_secs(5);

fn token(address: &str) -> String {
    format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(json!({"addr":address,"exp":4_102_444_800_u64}).to_string())
    )
}

#[derive(Clone, Default)]
struct Gate {
    armed: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl Gate {
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
    async fn wait(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
    async fn entered(&self) {
        tokio::time::timeout(WATCHDOG, self.entered.notified())
            .await
            .unwrap();
    }
}

#[derive(Clone, Default)]
struct FixtureState {
    reject_me: Arc<AtomicBool>,
    me: Gate,
    login: Gate,
    logout: Gate,
    balance: Gate,
    cookie_identity: Arc<parking_lot::Mutex<String>>,
}

struct Fixture {
    endpoint: url::Url,
    state: FixtureState,
    task: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fixture {
    async fn start() -> Self {
        let state = FixtureState::default();
        *state.cookie_identity.lock() = "cookie-a".into();
        let app = Router::new()
            .route("/v1/account/me", get(|State(state): State<FixtureState>, headers: HeaderMap| async move {
                let address = headers.get("authorization").and_then(|header| header.to_str().ok())
                    .and_then(|token| token.split('.').nth(1)).and_then(|raw| URL_SAFE_NO_PAD.decode(raw).ok())
                    .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                    .and_then(|raw| raw["addr"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| state.cookie_identity.lock().clone());
                state.me.wait().await;
                if state.reject_me.load(Ordering::SeqCst) {
                    (StatusCode::UNAUTHORIZED, Json(json!({"message":"session expired"})))
                } else {
                    (StatusCode::OK, Json(json!({"data":{"username":address,"walletAddress":address}})))
                }
            }))
            .route("/v1/account/nonce", post(|| async { Json(json!({"data":{"nonce":"fixture"}})) }))
            .route("/v1/account/login", post(|State(state): State<FixtureState>, Json(body): Json<Value>| async move {
                state.login.wait().await;
                let address = body["walletAddress"].as_str().unwrap();
                *state.cookie_identity.lock() = address.into();
                Json(json!({"data":{"token":token(address),"refreshToken":token("refresh")}}))
            }))
            .route("/v1/account/logout", post(|State(state): State<FixtureState>| async move {
                state.logout.wait().await;
                (StatusCode::UNAUTHORIZED, Json(json!({"message":"already signed out"})))
            }))
            .route("/v4/account/balance", get(|State(state): State<FixtureState>| async move {
                state.balance.wait().await;
                Json(json!({"data":{"sogni":"old-account-balance"}}))
            }))
            .route("/v1/chat/completions", post(|| async {
                (StatusCode::UNAUTHORIZED, Json(json!({"message":"session expired"})))
            }))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            endpoint,
            state,
            task,
        }
    }

    async fn client(&self, kind: AuthKind) -> SogniClient {
        let builder = SogniClient::builder();
        let builder = if kind == AuthKind::Cookies {
            builder.cookie_auth()
        } else {
            builder
        };
        builder
            .rest_endpoint(self.endpoint.clone())
            .disable_socket(true)
            .request_timeout(Duration::from_secs(30))
            .build()
            .await
            .unwrap()
    }
}

async fn install(client: &SogniClient, address: &str) {
    tokio::time::timeout(
        WATCHDOG,
        client.set_tokens(token(address), token("refresh")),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn explicit_token_switch_supersedes_pending_account_lookup_and_balance() {
    for balance in [false, true] {
        let fixture = Fixture::start().await;
        let client = fixture.client(AuthKind::Token).await;
        install(&client, "A").await;
        let gate = if balance {
            &fixture.state.balance
        } else {
            &fixture.state.me
        };
        gate.arm();
        let pending_client = client.clone();
        let pending = tokio::spawn(async move {
            if balance {
                pending_client.account.refresh_balance().await
            } else {
                pending_client.account.me().await
            }
        });
        gate.entered().await;
        install(&client, "B").await;
        assert!(matches!(
            tokio::time::timeout(WATCHDOG, pending)
                .await
                .unwrap()
                .unwrap(),
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(
            client.current_account().wallet_address().as_deref(),
            Some("B")
        );
        assert_ne!(
            client.current_account().balance()["sogni"],
            "old-account-balance"
        );
        gate.release.notify_one();
        client.close().await.unwrap();
    }
}

#[tokio::test]
async fn newer_token_installation_does_not_wait_for_or_roll_back_old_initial_hydration() {
    let fixture = Fixture::start().await;
    let client = fixture.client(AuthKind::Token).await;
    fixture.state.me.arm();
    let older = client.clone();
    let pending = tokio::spawn(async move { older.set_tokens(token("A"), token("refresh")).await });
    fixture.state.me.entered().await;
    install(&client, "B").await;
    assert!(
        tokio::time::timeout(WATCHDOG, pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(client.is_authenticated());
    assert_eq!(
        client.current_account().wallet_address().as_deref(),
        Some("B")
    );
    fixture.state.me.release.notify_one();
    client.close().await.unwrap();
}

#[cfg(feature = "wallet")]
#[tokio::test]
async fn late_login_response_cannot_replace_a_newer_completed_login() {
    let fixture = Fixture::start().await;
    let client = fixture.client(AuthKind::Token).await;
    fixture.state.login.arm();
    let older = client.clone();
    let pending =
        tokio::spawn(async move { older.account.login("older", "fixture password").await });
    fixture.state.login.entered().await;
    tokio::time::timeout(WATCHDOG, client.account.login("newer", "fixture password"))
        .await
        .unwrap()
        .unwrap();
    let current = client.current_account().wallet_address();
    assert!(current.is_some());
    assert!(
        tokio::time::timeout(WATCHDOG, pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    fixture.state.login.release.notify_one();
    assert_eq!(client.current_account().wallet_address(), current);
    assert!(client.is_authenticated());
    client.close().await.unwrap();
}

#[cfg(feature = "wallet")]
#[tokio::test]
async fn pending_cookie_check_cannot_block_or_restore_over_a_newer_login() {
    let fixture = Fixture::start().await;
    let client = fixture.client(AuthKind::Cookies).await;
    fixture.state.me.arm();
    let checking = client.clone();
    let pending = tokio::spawn(async move { checking.check_auth().await });
    fixture.state.me.entered().await;
    tokio::time::timeout(WATCHDOG, client.account.login("newer", "fixture password"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !tokio::time::timeout(WATCHDOG, pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    );
    let address = client.current_account().wallet_address().unwrap();
    assert_ne!(address, "cookie-a");
    assert!(client.is_authenticated());
    fixture.state.me.release.notify_one();
    assert!(client.check_auth().await.unwrap());
    assert_eq!(
        client.current_account().wallet_address().as_deref(),
        Some(address.as_str())
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn delayed_logout_401_cannot_sign_out_a_new_account_but_ordinary_401_succeeds() {
    let fixture = Fixture::start().await;
    let client = fixture.client(AuthKind::Token).await;
    install(&client, "A").await;
    fixture.state.logout.arm();
    let leaving = client.clone();
    let pending = tokio::spawn(async move { leaving.account.logout().await });
    fixture.state.logout.entered().await;
    install(&client, "B").await;
    assert!(
        tokio::time::timeout(WATCHDOG, pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(client.is_authenticated());
    assert_eq!(
        client.current_account().wallet_address().as_deref(),
        Some("B")
    );
    fixture.state.logout.release.notify_one();
    client.account.logout().await.unwrap();
    assert!(!client.is_authenticated());
    assert!(client.current_account().wallet_address().is_none());
    client.close().await.unwrap();
}

#[tokio::test]
async fn public_guarded_account_and_hosted_chat_preserve_their_own_401_status() {
    for hosted in [false, true] {
        let fixture = Fixture::start().await;
        let client = fixture.client(AuthKind::Token).await;
        install(&client, "A").await;
        fixture.state.reject_me.store(true, Ordering::SeqCst);
        let error = if hosted {
            client
                .chat
                .create_hosted_completion(
                    &json!({"model":"fixture","messages":[{"role":"user","content":"Hi"}]}),
                )
                .await
                .unwrap_err()
        } else {
            client.account.me().await.unwrap_err()
        };
        if hosted {
            assert!(matches!(error,Error::Chat(error) if error.status == Some(401)));
        } else {
            assert!(matches!(error,Error::Api(error) if error.status == 401));
        }
        assert!(!client.is_authenticated());
        client.close().await.unwrap();
    }
}
