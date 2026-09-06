use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::{Json, Router, http::StatusCode, routing::get};

use super::*;
use crate::SogniClient;

const ID: &str = "00000000-0000-4000-8000-000000000001";

async fn fixture(code: StatusCode) -> (SogniClient, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counts = calls.clone();
    let app = Router::new().route("/v2/projects/{id}", get(move || {
        let finished = counts.fetch_add(1, Ordering::SeqCst) > 0;
        async move {
            (code, Json(json!({"data":{"project":{
                "id":ID,"status":if finished {"canceled"} else {"processing"},"finished":finished,
            }}})))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = SogniClient::builder()
        .api_key("fixture")
        .rest_endpoint(Url::parse(&format!("http://{address}/")).unwrap())
        .defer_socket_start(true)
        .build()
        .await
        .unwrap();
    (client, calls, server)
}

#[tokio::test]
async fn cancellation_acknowledgment_waits_for_owner_status_to_finish() {
    let (client, calls, server) = fixture(StatusCode::OK).await;
    let status = tokio::time::timeout(
        Duration::from_secs(3),
        terminal_after_cancel(&client.projects, ID),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(status["status"], "canceled");
    assert_eq!(status["finished"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn missing_or_unavailable_status_never_confirms_cancellation() {
    for code in [
        StatusCode::NOT_FOUND,
        StatusCode::FORBIDDEN,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        let (client, calls, server) = fixture(code).await;
        assert!(terminal_after_cancel(&client.projects, ID).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        client.close().await.unwrap();
        server.abort();
    }
}
