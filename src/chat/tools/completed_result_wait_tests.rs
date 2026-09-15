use std::sync::Arc;

use axum::{Json, Router, routing::get};
use serde_json::json;
use tokio::sync::{Notify, watch};

use super::*;
use crate::{JobStatus, SogniClient};

#[tokio::test]
async fn completed_project_resolves_signed_result_after_queue_budget() {
    const PROJECT: &str = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    const IMAGE: &str = "BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB";
    const RESULT: &str = "https://media.sogni.ai/completed-fixture.png";
    let download_started = Arc::new(Notify::new());
    let (release_download, release) = watch::channel(false);
    let started = download_started.clone();
    let app = Router::new()
        .route(
            "/api/v1/artist/projects/sync",
            get(|| async {
                Json(json!({
                    "activeProjects":[],
                    "unclaimedCompletedProjects":[{
                        "id":PROJECT, "status":"completed", "finished":true,
                        "network":"fast", "imageCount":1,
                        "completedWorkerJobs":[{"imgID":IMAGE, "status":"jobCompleted"}]
                    }]
                }))
            }),
        )
        .route(
            "/v1/image/downloadUrl",
            get(move || {
                let started = started.clone();
                let mut release = release.clone();
                async move {
                    started.notify_one();
                    release.wait_for(|released| *released).await.unwrap();
                    Json(json!({"data":{"downloadUrl":RESULT}}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = SogniClient::builder()
        .app_id("completed-result-wait-fixture")
        .api_key("local-fixture")
        .rest_endpoint(format!("http://{address}/").parse().unwrap())
        .socket_endpoint(format!("ws://{address}/").parse().unwrap())
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(30))
        .build()
        .await
        .unwrap();
    let projects = client.projects.clone();
    let recovery = tokio::spawn(async move { projects.sync("completed-result-fixture").await });
    tokio::time::timeout(Duration::from_secs(5), download_started.notified())
        .await
        .unwrap();
    let project = client.projects.tracked_projects().pop().unwrap();
    assert_eq!(project.status(), ProjectStatus::Completed);
    assert_eq!(project.jobs()[0].status(), JobStatus::Completed);
    assert!(project.result_urls().is_empty());

    // Recovery already established terminal state. Its HTTP result lookup and
    // this waiter's lookup have their own request timeout, beyond the queue budget.
    tokio::time::pause();
    let wait = wait_for_tool_project(&project, Duration::from_secs(1));
    tokio::pin!(wait);
    assert!(futures_util::poll!(&mut wait).is_pending());
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(
        futures_util::poll!(&mut wait).is_pending(),
        "completed result retrieval must not enter the tool timeout/cancellation path"
    );
    release_download.send(true).unwrap();
    tokio::time::resume();
    let urls = tokio::time::timeout(Duration::from_secs(5), &mut wait)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(urls, vec![RESULT.to_owned()]);
    assert_eq!(project.status(), ProjectStatus::Completed);
    assert_eq!(project.jobs()[0].status(), JobStatus::Completed);
    recovery.await.unwrap().unwrap();
    client.close().await.unwrap();
    server.abort();
}
