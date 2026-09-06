use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::{Json, Router, http::StatusCode, routing::get};

use super::*;
use crate::SogniClient;

const ID: &str = "00000000-0000-4000-8000-000000000001";

async fn fixture(
    code: StatusCode,
    project: Value,
) -> (SogniClient, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let active_calls = Arc::new(AtomicUsize::new(0));
    let calls = active_calls.clone();
    let app = Router::new()
        .route(
            "/v2/projects/{id}",
            get(move || {
                let project = project.clone();
                async move { (code, Json(json!({"data":{"project": project}}))) }
            }),
        )
        .route(
            "/api/v1/artist/projects/active",
            get(move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Json(json!({"projects":[]})) }
            }),
        )
        .route(
            "/api/v1/artist/projects/sync",
            get(|| async { Json(json!({"activeProjects":[], "unclaimedCompletedProjects":[]})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = SogniClient::builder()
        .api_key("local-fixture")
        .rest_endpoint(Url::parse(&format!("http://{address}/")).unwrap())
        .socket_endpoint(Url::parse(&format!("ws://{address}/")).unwrap())
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(1))
        .build()
        .await
        .unwrap();
    (client, active_calls, server)
}

fn options() -> Option<ResolveMissingOptions> {
    Some(ResolveMissingOptions {
        attempts: 2,
        retry_delay: Duration::ZERO,
    })
}

#[tokio::test]
async fn lower_case_uuid_uses_the_case_sensitive_canonical_v2_route() {
    const LOWER: &str = "abcdefab-cdef-4abc-8abc-abcdefabcdef";
    let expected = LOWER.to_uppercase();
    let app = Router::new().route("/v2/projects/{id}", get(move |axum::extract::Path(id): axum::extract::Path<String>| {
        let expected = expected.clone();
        async move {
            if id != expected {
                return (StatusCode::NOT_FOUND, Json(json!({"error":102})));
            }
            (StatusCode::OK, Json(json!({"data":{"project":{"id":id,"status":"completed","finished":true}}})))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = SogniClient::builder()
        .api_key("local-fixture")
        .rest_endpoint(Url::parse(&format!("http://{address}/")).unwrap())
        .defer_socket_start(true)
        .build()
        .await
        .unwrap();
    assert_eq!(
        client.projects.get_status(LOWER).await.unwrap()["finished"],
        true
    );
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn v2_pending_queued_processing_are_active_without_a_registry_fallback() {
    for status in ["pending", "queued", "processing"] {
        let (client, calls, server) = fixture(
            StatusCode::OK,
            json!({
                "id":ID, "status":status, "finished":false,
            }),
        )
        .await;
        let resolved = client.projects.resolve_missing(&[ID], options()).await;
        assert_eq!(resolved.get(ID), Some(&ProjectResolution::Active));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        client.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn compact_failed_and_canceled_statuses_recover_as_terminal_without_jobs() {
    for (status, expected) in [
        ("failed", ProjectStatus::Failed),
        ("canceled", ProjectStatus::Canceled),
    ] {
        let (client, calls, server) = fixture(
            StatusCode::OK,
            json!({
                "id":ID, "status":status, "finished":true, "statusOnly":true,
                "workerJobs":[], "completedWorkerJobs":[],
            }),
        )
        .await;
        let resolved = client.projects.resolve_missing(&[ID], options()).await;
        assert!(matches!(
            resolved.get(ID),
            Some(ProjectResolution::Finished { .. })
        ));
        let recovered = client.projects.recover_project(ID).await.unwrap();
        assert_eq!(recovered.status(), expected);
        let completed = recovered
            .wait_for_completion(Some(Duration::from_millis(20)))
            .await;
        assert!(matches!(completed, Err(Error::Project(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        client.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn authorization_transport_and_malformed_statuses_remain_unknown() {
    let mut cases = vec![
        (StatusCode::FORBIDDEN, json!({})),
        (StatusCode::SERVICE_UNAVAILABLE, json!({})),
        (StatusCode::TOO_MANY_REQUESTS, json!({})),
    ];
    for project in [
        json!({"id":ID,"status":"processing","finished":true}),
        json!({"id":ID,"status":"canceled","finished":false}),
        json!({"id":ID,"status":"unknown","finished":true}),
        json!({"id":"different","status":"completed","finished":true}),
        json!({"id":ID,"status":"completed"}),
    ] {
        cases.push((StatusCode::OK, project));
    }
    for (code, project) in cases {
        let (client, calls, server) = fixture(code, project).await;
        let resolved = client.projects.resolve_missing(&[ID], options()).await;
        assert!(matches!(
            resolved.get(ID),
            Some(ProjectResolution::Unknown { .. })
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        client.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn repeated_v2_404_needs_a_valid_empty_registry_before_current_absence() {
    let (client, calls, server) = fixture(StatusCode::NOT_FOUND, json!({})).await;
    let resolved = client.projects.resolve_missing(&[ID], options()).await;
    assert_eq!(resolved.get(ID), Some(&ProjectResolution::Lost));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    client.close().await.unwrap();
    server.abort();
}
