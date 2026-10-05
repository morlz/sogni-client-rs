use std::sync::atomic::{AtomicUsize, Ordering};

use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    routing::get,
};

use super::*;
use crate::SogniClient;

const ID: &str = "abcdefab-cdef-4abc-8abc-abcdefabcdef";

async fn fixture(
    status: StatusCode,
    body: Value,
    retry_after: Option<&str>,
    roster_status: StatusCode,
    roster: Value,
) -> (
    SogniClient,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let status_calls = Arc::new(AtomicUsize::new(0));
    let active_calls = Arc::new(AtomicUsize::new(0));
    let status_counter = status_calls.clone();
    let active_counter = active_calls.clone();
    let mut headers = HeaderMap::new();
    if let Some(value) = retry_after {
        headers.insert("retry-after", value.parse().unwrap());
    }
    let roster_headers = headers.clone();
    let app = Router::new()
        .route(
            "/v2/projects/{id}",
            get(move || {
                status_counter.fetch_add(1, Ordering::SeqCst);
                let body = body.clone();
                let headers = headers.clone();
                async move { (status, headers, Json(body)) }
            }),
        )
        .route(
            "/api/v1/artist/projects/active",
            get(move || {
                active_counter.fetch_add(1, Ordering::SeqCst);
                let roster = roster.clone();
                let headers = roster_headers.clone();
                async move { (roster_status, headers, Json(roster)) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = SogniClient::builder()
        .app_id("local-recovery-advice-fixture")
        .api_key("local-fixture")
        .rest_endpoint(Url::parse(&format!("http://{address}/")).unwrap())
        .socket_endpoint(Url::parse(&format!("ws://{address}/")).unwrap())
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(1))
        .build()
        .await
        .unwrap();
    (client, status_calls, active_calls, server)
}

fn options() -> Option<ResolveMissingOptions> {
    Some(ResolveMissingOptions {
        attempts: 2,
        retry_delay: Duration::ZERO,
    })
}

#[tokio::test]
async fn unknown_http_verdict_retains_only_coarse_advice_without_another_lookup() {
    for (status, retry, header, expected) in [
        (StatusCode::FORBIDDEN, Value::Null, None, None),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Value::Null,
            Some("12"),
            Some(12),
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!(1.25),
            Some("12"),
            Some(2),
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!(-1),
            Some("12"),
            Some(12),
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            Value::Null,
            Some("18446744073709551616"),
            None,
        ),
    ] {
        let (client, status_calls, active_calls, server) = fixture(
            status,
            json!({
                "status": "error",
                "message": "fixture-private-diagnostic",
                "details": {"fixturePrivate": "fixture-private-diagnostic"},
                "retryAfter": retry,
            }),
            header,
            StatusCode::OK,
            json!({"projects":[]}),
        )
        .await;
        let report = client
            .projects
            .resolve_missing_with_advice(&[ID, ID], options())
            .await;
        assert_eq!(
            report.resolutions[ID],
            ProjectResolution::Unknown {
                error: "project status could not be verified".into(),
            }
        );
        assert_eq!(
            report.retry_advice[ID],
            ProjectRecoveryAdvice {
                status: status.as_u16(),
                retry_after_seconds: expected,
            }
        );
        let encoded = serde_json::to_value(&report).unwrap();
        assert!(!encoded.to_string().contains("fixture-private-diagnostic"));
        assert_eq!(encoded["retryAdvice"][ID]["status"], status.as_u16());
        assert_eq!(status_calls.load(Ordering::SeqCst), 1);
        assert_eq!(active_calls.load(Ordering::SeqCst), 0);
        client.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn exhausted_404_with_unavailable_roster_retains_the_roster_retry_advice() {
    let (client, status_calls, active_calls, server) = fixture(
        StatusCode::NOT_FOUND,
        json!({"error":102}),
        Some("7"),
        StatusCode::SERVICE_UNAVAILABLE,
        json!({"error":{"message":"fixture-private-diagnostic"}}),
    )
    .await;
    let report = client
        .projects
        .resolve_missing_with_advice(&[ID], options())
        .await;
    assert!(matches!(
        report.resolutions[ID],
        ProjectResolution::Unknown { .. }
    ));
    assert_eq!(
        report.retry_advice[ID],
        ProjectRecoveryAdvice {
            status: 503,
            retry_after_seconds: Some(7)
        }
    );
    assert_eq!(status_calls.load(Ordering::SeqCst), 2);
    assert_eq!(active_calls.load(Ordering::SeqCst), 1);
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn malformed_roster_does_not_prove_loss_or_invent_http_advice() {
    let (client, status_calls, active_calls, server) = fixture(
        StatusCode::NOT_FOUND,
        json!({"error":102}),
        Some("7"),
        StatusCode::OK,
        json!({"projects":[{"id":null}]}),
    )
    .await;
    let report = client
        .projects
        .resolve_missing_with_advice(&[ID], options())
        .await;
    assert!(matches!(
        report.resolutions[ID],
        ProjectResolution::Unknown { .. }
    ));
    assert!(report.retry_advice.is_empty());
    assert_eq!(status_calls.load(Ordering::SeqCst), 2);
    assert_eq!(active_calls.load(Ordering::SeqCst), 1);
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn legacy_resolution_map_and_unknown_serialization_remain_compatible() {
    let (client, status_calls, active_calls, server) = fixture(
        StatusCode::TOO_MANY_REQUESTS,
        json!({"error":{"retryAfter":4}}),
        None,
        StatusCode::OK,
        json!({"projects":[]}),
    )
    .await;
    let resolutions = client.projects.resolve_missing(&[ID], options()).await;
    assert_eq!(
        serde_json::to_value(&resolutions[ID]).unwrap(),
        json!({"state":"unknown", "error":"project status could not be verified"})
    );
    assert_eq!(status_calls.load(Ordering::SeqCst), 1);
    assert_eq!(active_calls.load(Ordering::SeqCst), 0);
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn closed_account_session_is_unknown_without_cross_session_advice() {
    let (client, status_calls, active_calls, server) = fixture(
        StatusCode::TOO_MANY_REQUESTS,
        json!({"error":{"retryAfter":4}}),
        None,
        StatusCode::OK,
        json!({"projects":[]}),
    )
    .await;
    client.close().await.unwrap();
    let report = client
        .projects
        .resolve_missing_with_advice(&[ID], options())
        .await;
    assert!(matches!(
        report.resolutions[ID],
        ProjectResolution::Unknown { .. }
    ));
    assert!(report.retry_advice.is_empty());
    assert_eq!(status_calls.load(Ordering::SeqCst), 0);
    assert_eq!(active_calls.load(Ordering::SeqCst), 0);
    server.abort();
}
