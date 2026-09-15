use super::*;
use crate::{AuthKind, auth::AuthManager, transport::HttpClients};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone)]
struct Fixture {
    base: String,
    mode: u16,
    prepares: Arc<AtomicUsize>,
    binds: Arc<AtomicUsize>,
    auth: AuthManager,
}

fn saved() -> Value {
    json!({"id":"asset", "name":"source", "bytes":3, "contentType":"image/png",
        "state":"ready", "createdAt":1, "expiresAt":9999999999999_u64})
}

async fn fixture(mode: u16) -> (ReusableUploads, Fixture, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let http = HttpClients::build(Duration::from_secs(2)).unwrap();
    let auth = AuthManager::new(
        AuthKind::ApiKey,
        base.parse().unwrap(),
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("fixture-secret").unwrap();
    let rest = RestClient::new(
        base.parse().unwrap(),
        auth.clone(),
        http,
        Duration::from_secs(2),
    );
    let state = Fixture {
        base,
        mode,
        prepares: Arc::new(AtomicUsize::new(0)),
        binds: Arc::new(AtomicUsize::new(0)),
        auth,
    };
    let app = Router::new()
        .route(
            "/v1/assets/capabilities",
            get(|| async { Json(json!({"data":{"enabled":true}})) }),
        )
        .route(
            "/v1/assets/prepare",
            post(
                |State(s): State<Fixture>, Json(body): Json<Value>| async move {
                    s.prepares.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(body["sha256"], format!("{:x}", Sha256::digest(b"png")));
                    if s.mode == 401 {
                        s.auth.clear();
                    }
                    if matches!(s.mode, 403 | 409) {
                        return (
                            StatusCode::from_u16(s.mode).unwrap(),
                            Json(json!({
                    "message":"Your saved upload library is full."})),
                        );
                    }
                    let mut value = saved();
                    value["state"] = json!("uploading");
                    value["uploadUrl"] = json!(format!("{}upload", s.base));
                    value["uploadHeaders"] =
                        json!({"content-type":"image/png", "if-none-match":"*"});
                    (StatusCode::OK, Json(json!({"data":value})))
                },
            ),
        )
        .route(
            "/upload",
            put(|request: Request| async move {
                assert!(request.headers().get("api-key").is_none());
                assert!(request.headers().get("authorization").is_none());
                assert!(request.headers().get("cookie").is_none());
                assert_eq!(request.headers()["if-none-match"], "*");
                let body = axum::body::to_bytes(request.into_body(), 100)
                    .await
                    .unwrap();
                assert_eq!(body.as_ref(), b"png");
                StatusCode::PRECONDITION_FAILED
            }),
        )
        .route(
            "/v1/assets/asset/finalize",
            post(|State(s): State<Fixture>| async move {
                if s.mode == 410 {
                    (
                        StatusCode::GONE,
                        Json(json!({"message":"verification failed"})),
                    )
                } else {
                    (StatusCode::OK, Json(json!({"data":saved()})))
                }
            }),
        )
        .route(
            "/v1/assets/asset/bind",
            post(
                |State(s): State<Fixture>, Json(body): Json<Value>| async move {
                    s.binds.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(body, json!({"projectId":"PROJECT", "type":"startingImage"}));
                    Json(json!({"data":{}}))
                },
            ),
        )
        .route(
            "/v1/assets/asset",
            delete(|| async { Json(json!({"data":{}})) }),
        )
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (ReusableUploads::new(rest), state, server)
}

async fn try_png(assets: &ReusableUploads) -> Result<bool> {
    assets
        .try_bind(
            Bytes::from_static(b"png"),
            Some("image/png"),
            "source",
            &SavedUploadBinding {
                project_id: "PROJECT".into(),
                asset_type: "startingImage".into(),
                id: None,
            },
        )
        .await
}

#[tokio::test]
async fn saved_upload_verifies_write_once_response_and_binds_without_credentials() {
    let (assets, state, server) = fixture(200).await;
    assert!(try_png(&assets).await.unwrap());
    assert_eq!(state.binds.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
async fn automatic_preparation_refusal_falls_back_and_quota_cools_down() {
    for mode in [403, 409] {
        let (assets, state, server) = fixture(mode).await;
        assert!(!try_png(&assets).await.unwrap());
        assert!(!try_png(&assets).await.unwrap());
        assert_eq!(
            state.prepares.load(Ordering::SeqCst),
            if mode == 409 { 1 } else { 2 }
        );
        assert_eq!(state.binds.load(Ordering::SeqCst), 0);
        if mode == 409 {
            assets.remove("asset").await.unwrap();
            assert!(!try_png(&assets).await.unwrap());
            assert_eq!(state.prepares.load(Ordering::SeqCst), 2);
        }
        server.abort();
    }
}

#[tokio::test]
async fn finalized_failures_and_account_changes_never_fall_back_or_bind() {
    for mode in [410, 401] {
        let (assets, state, server) = fixture(mode).await;
        let error = try_png(&assets).await.unwrap_err();
        if mode == 410 {
            assert!(matches!(error, Error::Api(ref e) if e.status == 410));
        } else {
            assert!(error.to_string().contains("account changed"));
        }
        assert_eq!(state.binds.load(Ordering::SeqCst), 0);
        server.abort();
    }
}

#[tokio::test]
async fn authentication_changes_clear_cached_eligibility_and_quota() {
    let (assets, state, server) = fixture(409).await;
    assert!(!try_png(&assets).await.unwrap());
    state.auth.authenticate_api_key("second-account").unwrap();
    assert!(!try_png(&assets).await.unwrap());
    assert_eq!(state.prepares.load(Ordering::SeqCst), 2);
    server.abort();
}
