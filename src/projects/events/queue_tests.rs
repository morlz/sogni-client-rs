use super::*;
use crate::SogniClient;

fn reason() -> Value {
    json!({"reason":"concurrency_limit","message":"Your included video slots are in use.","mediaType":"video","paymentModel":"subscription","subscriptionTier":"unlimited"})
}
fn row(index: u64) -> Value {
    json!({"jobIndex":index,"waitingReason":reason()})
}
fn queue() -> Value {
    json!({"waitingReason":reason(),"jobWaitingReasons":[row(0),row(1)]})
}
fn tracked(client: &SogniClient) -> Project {
    let project = Project::new(
        "PROJECT".into(),
        json!({"type":"video","numberOfMedia":2,"steps":10}),
        false,
        Arc::downgrade(&client.projects.inner),
    );
    client
        .projects
        .inner
        .projects
        .write()
        .insert(project.id(), project.clone());
    project
}

#[test]
fn queue_normalization_retains_only_the_public_contract() {
    let mut raw = reason();
    raw["privateField"] = json!(123);
    raw["modelFamily"] = json!("other");
    raw["paymentModel"] = json!(["subscription"]);
    let parsed = queue::normalize_waiting_reason(&raw).unwrap();
    assert!(parsed.payment_model.is_none());
    assert!(parsed.model_family.is_none());
    assert!(
        serde_json::to_value(parsed)
            .unwrap()
            .get("privateField")
            .is_none()
    );
    for message in [" ".into(), "😀".repeat(301)] {
        raw["message"] = json!(message);
        assert!(queue::normalize_waiting_reason(&raw).is_none());
    }
    raw = reason();
    raw["reason"] = json!("future_reason");
    assert!(queue::normalize_waiting_reason(&raw).is_none());
    assert_eq!(
        queue::normalize_job_waiting_reasons(&json!([row(0), row(0), row(1)]), 2).len(),
        1
    );
    assert_eq!(
        queue::normalize_job_waiting_reasons(&json!([row(9), row(1)]), 2)[0].job_index,
        1
    );
}

#[tokio::test]
async fn current_queue_replaces_rows_without_phantom_jobs_or_runtime_changes() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let project = tracked(&client);
    let mut events = client.projects.subscribe();
    project.receive_queue(&queue());
    let changed = events.try_recv().unwrap();
    assert_eq!(changed.name, "queueChanged");
    assert_eq!(
        changed.data["waitingReason"],
        json!(project.waiting_reason())
    );
    assert!(project.jobs().is_empty());
    project.receive_queue(&queue());
    assert!(events.try_recv().is_err());
    state::handle_job_state(
        &client.projects.inner,
        &json!({"jobID":"PROJECT","type":"queued","queuePosition":2}),
    );
    assert!(project.waiting_reason().is_some());
    state::handle_job_state(
        &client.projects.inner,
        &json!({"jobID":"PROJECT","imgID":"RUNNING","jobIndex":1,"type":"jobStarted"}),
    );
    let running = project.job("RUNNING").unwrap();
    let deadline = running.processing_deadline();
    assert!(deadline.is_some());
    project.receive_queue(&queue());
    assert_eq!(
        project
            .job_waiting_reasons()
            .iter()
            .map(|row| row.job_index)
            .collect::<Vec<_>>(),
        vec![0]
    );
    assert!(running.waiting_reason().is_none());
    assert_eq!(running.processing_deadline(), deadline);
    let pending = project.ensure_job("pending-spelling");
    pending.update(
        |state| {
            state.extra.insert("jobIndex".into(), json!(0));
        },
        &["jobIndex"],
    );
    project.receive_queue(&json!({"waitingReason":reason(),"jobWaitingReasons":[{"imgID":"pending-spelling","jobIndex":0,"waitingReason":reason()}]}));
    assert!(pending.waiting_reason().is_some());
    assert_eq!(
        project.job_waiting_reasons()[0].img_id.as_deref(),
        Some("pending-spelling")
    );
    running.suspend_processing_deadline();
    project.receive_queue(&json!({"waitingReason":null,"jobWaitingReasons":[]}));
    assert!(pending.waiting_reason().is_none());
    assert!(running.processing_deadline().is_none());
    project.receive_queue(&queue());
    project.update(|state| state.status = ProjectStatus::Completed, &["status"]);
    project.receive_queue(&queue());
    assert!(project.waiting_reason().is_none());
    assert!(project.job_waiting_reasons().is_empty());
    client.close().await.unwrap();
}

#[tokio::test]
async fn stale_recovery_cannot_undo_live_retry_or_clear_but_still_collects_terminal_results() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let project = tracked(&client);
    state::handle_job_state(
        &client.projects.inner,
        &json!({"jobID":"PROJECT","imgID":"OLD","jobIndex":0,"type":"jobStarted"}),
    );
    let revision = project.queue_revision();
    project.retry_job(&json!({"imgID":"OLD","jobIndex":0}));
    project.receive_queue(&json!({"waitingReason":reason(),"jobWaitingReasons":[row(0)]}));
    replay_recovered_at_revision(
        &project,
        &json!({"status":"processing","workerJobs":[{"imgID":"STALE","jobIndex":0,"status":"jobStarted"}],"completedWorkerJobs":[{"imgID":"DONE","jobIndex":1,"status":"jobCompleted","resultUrl":"https://example.test/result.mp4"}],"waitingReason":null,"jobWaitingReasons":[]}),
        false,
        revision,
    );
    assert!(project.job("STALE").is_none());
    assert_eq!(project.job("OLD").unwrap().status(), JobStatus::Pending);
    assert_eq!(project.job("DONE").unwrap().status(), JobStatus::Completed);
    assert_eq!(project.job_waiting_reasons()[0].job_index, 0);
    let revision = project.queue_revision();
    project.invalidate_queue();
    replay_recovered_at_revision(&project, &queue(), false, revision);
    assert!(project.waiting_reason().is_none());
    client.close().await.unwrap();
}

#[tokio::test]
async fn delayed_sync_uses_request_start_queue_revision_including_projects_created_during_request()
{
    use axum::{Json, Router, routing::get};
    for created_during_request in [false, true] {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let notify = entered.clone();
        let gate = release.clone();
        let app = Router::new().route("/api/v1/artist/projects/sync",get(move || {
            let notify = notify.clone(); let gate = gate.clone();
            async move {
                notify.notify_one(); gate.notified().await;
                Json(json!({"activeProjects":[{"id":"PROJECT","status":"queued","workerJobs":[],"completedWorkerJobs":[],"waitingReason":reason(),"jobWaitingReasons":[row(0)]}],"unclaimedCompletedProjects":[]}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = SogniClient::builder()
            .app_id("queue-fixture")
            .api_key("synthetic-queue-fixture")
            .defer_socket_start(true)
            .rest_endpoint(endpoint.parse().unwrap())
            .socket_endpoint(endpoint.replace("http:", "ws:").parse().unwrap())
            .build()
            .await
            .unwrap();
        let mut project = (!created_during_request).then(|| tracked(&client));
        let api = client.projects.clone();
        let sync = tokio::spawn(async move { api.sync("fixture").await });
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
        let project = project.get_or_insert_with(|| tracked(&client));
        project.receive_queue(&json!({"waitingReason":null,"jobWaitingReasons":[]}));
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), sync)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(project.waiting_reason().is_none());
        assert!(project.job_waiting_reasons().is_empty());
        client.close().await.unwrap();
        server.abort();
    }
}
