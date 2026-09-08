use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{Json, Router, extract::State, routing::get};
use serde_json::json;
use url::Url;

use super::SogniClient;

mod project_wire;
mod project_wire_server;
mod rest_only;
mod submission_phase;
mod utility_projects;
mod wire_auth;

#[tokio::test]
async fn deferred_catalogue_auth_uses_http_without_opening_realtime_session() {
    let upgrades = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/",
            get(|State(count): State<Arc<AtomicUsize>>| async move {
                count.fetch_add(1, Ordering::SeqCst);
                "unexpected realtime handshake"
            }),
        )
        .route(
            "/api/v1/models/list",
            get(|| async { Json(json!([{"id":"fixture-model"}])) }),
        )
        .with_state(upgrades.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve fixture");
    });
    let client = SogniClient::builder()
        .app_id("local-parity-fixture")
        .api_key("fixture-key")
        .socket_endpoint(Url::parse(&format!("ws://{address}/")).expect("fixture endpoint"))
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(2))
        .build()
        .await
        .expect("deferred client");
    let models = client
        .projects
        .get_supported_models(true)
        .await
        .expect("HTTP models");
    assert_eq!(models, vec![json!({"id":"fixture-model"})]);
    assert!(client.is_authenticated());
    assert!(!client.is_socket_connected());
    client.close().await.expect("close client");
    assert_eq!(upgrades.load(Ordering::SeqCst), 0);
    server.abort();
}
