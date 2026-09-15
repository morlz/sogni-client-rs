use axum::{Json, Router, routing::get};
use parking_lot::RwLock;
use serde_json::{Value, json};
use std::sync::Arc;

use super::*;
use crate::{JobStatus, SogniClient};

const PROJECT_ID: &str = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";

async fn fixture() -> (
    SogniClient,
    Project,
    Arc<RwLock<Value>>,
    tokio::task::JoinHandle<()>,
) {
    fixture_with_status("processing").await
}

async fn fixture_with_status(
    status: &str,
) -> (
    SogniClient,
    Project,
    Arc<RwLock<Value>>,
    tokio::task::JoinHandle<()>,
) {
    let snapshot = Arc::new(RwLock::new(json!({
        "activeProjects":[{
            "id":PROJECT_ID, "status":status, "network":"fast", "imageCount":1,
            "workerJobs":[{"imgID":"BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB", "status":"jobStarted", "jobIndex":0}]
        }],
        "unclaimedCompletedProjects":[]
    })));
    let response = snapshot.clone();
    let app = Router::new().route(
        "/api/v1/artist/projects/sync",
        get(move || {
            let snapshot = response.read().clone();
            async move { Json(snapshot) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = SogniClient::builder()
        .app_id("processing-deadline-fixture")
        .api_key("local-fixture")
        .rest_endpoint(format!("http://{address}/").parse().unwrap())
        .socket_endpoint(format!("ws://{address}/").parse().unwrap())
        .defer_socket_start(true)
        .build()
        .await
        .unwrap();
    let project = client.projects.recover_project(PROJECT_ID).await.unwrap();
    (client, project, snapshot, server)
}

async fn advance_past(deadline: Instant) {
    // Tokio's timer wheel rounds to milliseconds.
    tokio::time::advance(
        deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
    )
    .await;
}

#[tokio::test]
async fn recovered_processing_without_events_has_a_finite_backstop() {
    let (client, project, _, server) = fixture().await;
    assert_eq!(project.jobs()[0].status(), JobStatus::Processing);
    let deadline = processing_deadline(&project).unwrap();
    tokio::time::pause();
    let wait = wait_for_tool_project(&project, Duration::from_secs(90));
    tokio::pin!(wait);
    assert!(futures_util::poll!(&mut wait).is_pending());
    tokio::time::advance(Duration::from_secs(91)).await;
    assert!(
        futures_util::poll!(&mut wait).is_pending(),
        "queue budget must not cancel processing"
    );
    advance_past(deadline).await;
    assert!(matches!(
        futures_util::poll!(&mut wait),
        std::task::Poll::Ready(Err(Error::Timeout(_)))
    ));
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn recovered_active_parent_uses_started_child_processing_budget() {
    let (client, project, _, server) = fixture_with_status("active").await;
    assert_eq!(project.status(), ProjectStatus::Queued);
    assert_eq!(project.jobs()[0].status(), JobStatus::Processing);
    let deadline = project.jobs()[0].processing_deadline().unwrap();
    tokio::time::pause();
    let wait = wait_for_tool_project(&project, Duration::from_millis(1));
    tokio::pin!(wait);
    assert!(futures_util::poll!(&mut wait).is_pending());
    tokio::time::advance(Duration::from_millis(2)).await;
    assert!(
        futures_util::poll!(&mut wait).is_pending(),
        "generic active recovery must not apply the queue budget to a started child"
    );
    advance_past(deadline).await;
    assert!(matches!(
        futures_util::poll!(&mut wait),
        std::task::Poll::Ready(Err(Error::Timeout(_)))
    ));
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn replacement_ready_with_old_timer_keeps_its_own_budget() {
    let (client, project, snapshot, server) = fixture().await;
    let old_deadline = processing_deadline(&project).unwrap();
    tokio::time::pause();
    let wait = wait_for_tool_project(&project, Duration::from_secs(90));
    tokio::pin!(wait);
    assert!(futures_util::poll!(&mut wait).is_pending());
    tokio::time::advance(Duration::from_secs(1700)).await;
    snapshot.write()["activeProjects"][0]["workerJobs"][0]["imgID"] =
        json!("CCCCCCCC-CCCC-4CCC-8CCC-CCCCCCCCCCCC");
    // Local HTTP needs the live I/O clock; do not poll the waiter until both
    // the replacement notification and its departed worker's timer are ready.
    tokio::time::resume();
    client.projects.sync("replacement-fixture").await.unwrap();
    tokio::time::pause();
    let new_deadline = processing_deadline(&project).unwrap();
    assert!(new_deadline > old_deadline);
    advance_past(old_deadline).await;
    assert!(
        futures_util::poll!(&mut wait).is_pending(),
        "an expired old attempt must not cancel its replacement"
    );
    advance_past(new_deadline).await;
    assert!(matches!(
        futures_util::poll!(&mut wait),
        std::task::Poll::Ready(Err(Error::Timeout(_)))
    ));
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn requeue_invalidates_processing_budget_without_changing_child_status() {
    let (client, project, snapshot, server) = fixture().await;
    let old_deadline = processing_deadline(&project).unwrap();
    tokio::time::pause();
    let wait = wait_for_tool_project(&project, Duration::from_secs(90));
    tokio::pin!(wait);
    assert!(futures_util::poll!(&mut wait).is_pending());
    snapshot.write()["activeProjects"][0]["status"] = json!("queued");
    tokio::time::resume();
    client.projects.sync("requeue-fixture").await.unwrap();
    tokio::time::pause();
    assert_eq!(project.jobs()[0].status(), JobStatus::Processing);
    assert!(processing_deadline(&project).is_none());
    advance_past(old_deadline).await;
    assert!(futures_util::poll!(&mut wait).is_pending());
    tokio::time::advance(Duration::from_secs(91)).await;
    assert!(matches!(
        futures_util::poll!(&mut wait),
        std::task::Poll::Ready(Err(Error::Timeout(_)))
    ));
    client.close().await.unwrap();
    server.abort();
}
