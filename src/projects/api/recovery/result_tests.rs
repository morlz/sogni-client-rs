use std::sync::atomic::{AtomicUsize, Ordering};

use axum::{Json, Router, http::StatusCode, routing::get};

use super::*;
use crate::SogniClient;

const ID: &str = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
const IMAGE: &str = "BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB";

fn completed_record() -> Value {
    json!({
        "id": ID, "status": "completed", "finished": true, "imageCount": 1,
        "workerJobs": [],
        "completedWorkerJobs": [{"imgID": IMAGE, "status": "jobCompleted"}]
    })
}

async fn fixture(
    snapshot: Value,
    status_available: bool,
) -> (
    SogniClient,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let sync_calls = Arc::new(AtomicUsize::new(0));
    let status_calls = Arc::new(AtomicUsize::new(0));
    let sync_counter = sync_calls.clone();
    let status_counter = status_calls.clone();
    let app = Router::new()
        .route(
            "/api/v1/artist/projects/sync",
            get(move || {
                sync_counter.fetch_add(1, Ordering::SeqCst);
                let snapshot = snapshot.clone();
                async move { Json(snapshot) }
            }),
        )
        .route(
            "/v2/projects/{id}",
            get(move || {
                status_counter.fetch_add(1, Ordering::SeqCst);
                async move {
                    if status_available {
                        (
                            StatusCode::OK,
                            Json(json!({"data":{"project":completed_record()}})),
                        )
                    } else {
                        (StatusCode::NOT_FOUND, Json(json!({"error":102})))
                    }
                }
            }),
        )
        .route(
            "/api/v1/artist/projects/active",
            get(|| async { Json(json!({"projects":[]})) }),
        )
        .route(
            "/v1/image/downloadUrl",
            get(|| async {
                Json(json!({"data":{"downloadUrl":"https://media.sogni.ai/fixture.png"}}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = SogniClient::builder()
        .api_key("local-fixture")
        .rest_endpoint(Url::parse(&format!("http://{address}/")).unwrap())
        .socket_endpoint(Url::parse(&format!("ws://{address}/")).unwrap())
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(1))
        .build()
        .await
        .unwrap();
    (client, sync_calls, status_calls, server)
}

fn tracked(client: &SogniClient, status: ProjectStatus, child: Option<JobStatus>) -> Project {
    let project = Project::new(
        ID.into(),
        json!({"type":"image", "numberOfMedia":1}),
        false,
        Arc::downgrade(&client.projects.inner),
    );
    project.update(
        |state| {
            state.status = status;
            state.started_at = Utc::now() - chrono::Duration::minutes(5);
        },
        &["status"],
    );
    if let Some(status) = child {
        project
            .ensure_job(IMAGE)
            .update(|state| state.status = status, &["status"]);
    }
    client
        .projects
        .inner
        .projects
        .write()
        .insert(ID.into(), project.clone());
    project
}

#[tokio::test]
async fn completed_parent_recovers_missing_result_in_place_before_socket_sync() {
    let (client, sync_calls, status_calls, server) = fixture(json!({}), true).await;
    let original = tracked(
        &client,
        ProjectStatus::Completed,
        Some(JobStatus::Processing),
    );
    let original_child = original.jobs()[0].clone();
    let recovered = client.projects.recover_project(ID).await.unwrap();
    assert_eq!(recovered.status(), ProjectStatus::Completed);
    assert_eq!(original_child.status(), JobStatus::Completed);
    assert!(original_child.has_result_media());
    assert_eq!(
        original
            .wait_for_completion(Some(Duration::from_millis(20)))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(sync_calls.load(Ordering::SeqCst), 0);
    assert_eq!(status_calls.load(Ordering::SeqCst), 1);
    client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn sync_recovers_incomplete_terminal_children_from_snapshot_or_durable_status() {
    for snapshot_contains_result in [true, false] {
        let records = if snapshot_contains_result {
            vec![completed_record()]
        } else {
            vec![]
        };
        let (client, _, status_calls, server) = fixture(
            json!({
                "activeProjects":[], "unclaimedCompletedProjects":records,
            }),
            true,
        )
        .await;
        let original = tracked(&client, ProjectStatus::Completed, None);
        client
            .projects
            .sync("result-recovery-fixture")
            .await
            .unwrap();
        assert_eq!(original.jobs().len(), 1);
        assert!(original.jobs()[0].has_result_media());
        assert!(original.jobs()[0].result_url().is_some());
        assert_eq!(
            status_calls.load(Ordering::SeqCst),
            usize::from(!snapshot_contains_result)
        );
        client.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn failed_and_canceled_zero_output_projects_do_not_request_successful_results() {
    for status in [ProjectStatus::Failed, ProjectStatus::Canceled] {
        let (client, _, status_calls, server) = fixture(
            json!({
                "activeProjects":[], "unclaimedCompletedProjects":[],
            }),
            true,
        )
        .await;
        let original = tracked(&client, status, None);
        let recovered = client.projects.recover_project(ID).await.unwrap();
        assert_eq!(recovered.status(), status);
        assert_eq!(original.status(), status);
        assert!(recovered.jobs().is_empty());
        assert_eq!(status_calls.load(Ordering::SeqCst), 0);
        client.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn missing_durable_result_never_erases_observed_project_completion() {
    let (client, _, status_calls, server) = fixture(
        json!({
            "activeProjects":[], "unclaimedCompletedProjects":[],
        }),
        false,
    )
    .await;
    let original = tracked(
        &client,
        ProjectStatus::Completed,
        Some(JobStatus::Processing),
    );
    let result = client
        .projects
        .sync("expired-result-fixture")
        .await
        .unwrap();
    assert_eq!(original.status(), ProjectStatus::Completed);
    assert!(original.snapshot().error.is_none());
    assert_eq!(result["lost"], json!([]));
    assert_eq!(result["unverified"], json!([ID]));
    assert_eq!(
        status_calls.load(Ordering::SeqCst),
        MISSING_PROJECT_ATTEMPTS
    );
    client.close().await.unwrap();
    server.abort();
}
