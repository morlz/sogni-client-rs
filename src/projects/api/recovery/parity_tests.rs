use std::sync::atomic::{AtomicUsize, Ordering};

use axum::{Json, Router, http::StatusCode, routing::get};

use super::*;
use crate::SogniClient;

const ID: &str = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
const IMAGE: &str = "BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB";

#[derive(Default)]
struct Lookup {
    entered: Notify,
    release: Notify,
    calls: AtomicUsize,
}

async fn fixture(
    status: StatusCode,
    raw: Value,
    hold: bool,
) -> (SogniClient, Arc<Lookup>, tokio::task::JoinHandle<()>) {
    let lookup = Arc::new(Lookup::default());
    let control = lookup.clone();
    let app = Router::new()
        .route(
            "/api/v1/artist/projects/sync",
            get(|| async { Json(json!({"activeProjects":[],"unclaimedCompletedProjects":[]})) }),
        )
        .route(
            "/v2/projects/{id}",
            get(move || {
                let raw = raw.clone();
                let control = control.clone();
                async move {
                    control.calls.fetch_add(1, Ordering::SeqCst);
                    control.entered.notify_one();
                    if hold {
                        control.release.notified().await;
                    }
                    (status, Json(json!({"data":{"project":raw}})))
                }
            }),
        )
        .route(
            "/api/v1/artist/projects/active",
            get(|| async { Json(json!({"projects":[]})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = SogniClient::builder()
        .app_id("local-recovery-parity-fixture")
        .api_key("local-fixture")
        .rest_endpoint(Url::parse(&format!("http://{address}/")).unwrap())
        .socket_endpoint(Url::parse(&format!("ws://{address}/")).unwrap())
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(2))
        .build()
        .await
        .unwrap();
    (client, lookup, server)
}

fn tracked(client: &SogniClient) -> Project {
    let project = Project::new(
        ID.into(),
        json!({
            "type":"image", "modelId":"fixture-model", "numberOfMedia":1,
            "steps":20, "numberOfPreviews":3, "positivePrompt":"fixture prompt",
        }),
        false,
        Arc::downgrade(&client.projects.inner),
    );
    project.update(
        |state| {
            state.status = ProjectStatus::Processing;
            state.started_at = Utc::now() - chrono::Duration::minutes(5);
        },
        &["status"],
    );
    project
        .ensure_job(IMAGE)
        .update(|state| state.status = JobStatus::Processing, &["status"]);
    client
        .projects
        .inner
        .projects
        .write()
        .insert(ID.into(), project.clone());
    project
}

#[tokio::test]
async fn compact_terminal_status_settles_tracked_waiters_without_erasing_generation_params() {
    for (status, expected) in [
        ("failed", ProjectStatus::Failed),
        ("canceled", ProjectStatus::Canceled),
    ] {
        let (client, lookup, server) = fixture(
            StatusCode::OK,
            json!({
                "id":ID, "status":status, "finished":true, "statusOnly":true,
                "workerJobs":[], "completedWorkerJobs":[], "reason":"fixture refusal",
            }),
            false,
        )
        .await;
        let project = tracked(&client);
        let original_params = project.snapshot().params;
        let child = project.jobs()[0].clone();
        let mut waiter = std::pin::pin!(project.wait_for_completion(Some(Duration::from_secs(2))));
        assert!(futures_util::poll!(&mut waiter).is_pending());
        let result = client.projects.sync("compact-status-parity").await.unwrap();
        assert_eq!(project.status(), expected);
        assert!(child.status().is_finished());
        assert_eq!(project.snapshot().params, original_params);
        let Error::Project(error) = waiter.await.unwrap_err() else {
            panic!("expected terminal project failure")
        };
        assert_eq!(error.message, "fixture refusal");
        assert_eq!(result["lost"], json!([]));
        assert_eq!(lookup.calls.load(Ordering::SeqCst), 1);
        client.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn inconclusive_status_does_not_cancel_an_existing_generation() {
    for status in [
        StatusCode::FORBIDDEN,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let (client, lookup, server) = fixture(status, json!({}), false).await;
        let project = tracked(&client);
        let result = client.projects.sync("unknown-status-parity").await.unwrap();
        assert_eq!(project.status(), ProjectStatus::Processing);
        assert_eq!(project.jobs()[0].status(), JobStatus::Processing);
        assert!(project.snapshot().error.is_none());
        assert_eq!(result["lost"], json!([]));
        assert_eq!(result["unverified"], json!([ID]));
        assert_eq!(lookup.calls.load(Ordering::SeqCst), 1);
        client.close().await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn terminal_live_state_wins_over_a_late_recovery_verdict() {
    for (code, raw) in [
        (StatusCode::SERVICE_UNAVAILABLE, json!({})),
        (
            StatusCode::OK,
            json!({"id":ID,"status":"failed","finished":true,"reason":"stale refusal"}),
        ),
    ] {
        let (client, lookup, server) = fixture(code, raw, true).await;
        let project = tracked(&client);
        let api = client.projects.clone();
        let syncing = tokio::spawn(async move { api.sync("completion-race-parity").await });
        tokio::time::timeout(Duration::from_secs(2), lookup.entered.notified())
            .await
            .unwrap();
        project.jobs()[0].update(
            |state| {
                state.status = JobStatus::Completed;
                state.result_url = Some("https://media.sogni.ai/fixture.png".into());
            },
            &["status", "resultUrl"],
        );
        project.update(|state| state.status = ProjectStatus::Completed, &["status"]);
        lookup.release.notify_one();
        let result = syncing.await.unwrap().unwrap();
        assert_eq!(project.status(), ProjectStatus::Completed);
        assert!(project.snapshot().error.is_none());
        assert_eq!(project.jobs()[0].status(), JobStatus::Completed);
        assert_eq!(
            project
                .wait_for_completion(Some(Duration::from_millis(20)))
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(result["lost"], json!([]));
        assert_eq!(result["unverified"], json!([]));
        assert_eq!(lookup.calls.load(Ordering::SeqCst), 1);
        client.close().await.unwrap();
        server.abort();
    }
}
