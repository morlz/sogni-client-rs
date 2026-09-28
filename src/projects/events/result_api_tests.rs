use super::*;
use crate::SogniClient;
use axum::{
    Json, Router,
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Default)]
struct FixtureState {
    replies: parking_lot::Mutex<HashMap<String, (StatusCode, Value)>>,
    calls: parking_lot::Mutex<Vec<(String, BTreeMap<String, String>)>>,
    hold: parking_lot::Mutex<Option<String>>,
    entered: Notify,
    release: Notify,
}
struct Fixture {
    client: SogniClient,
    state: Arc<FixtureState>,
    server: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start() -> Self {
        let state = Arc::new(FixtureState::default());
        let router = Router::new().fallback(serve).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = SogniClient::builder()
            .app_id("result-fixture")
            .api_key("synthetic-result-fixture")
            .defer_socket_start(true)
            .rest_endpoint(endpoint.parse().unwrap())
            .socket_endpoint(endpoint.replace("http:", "ws:").parse().unwrap())
            .build()
            .await
            .unwrap();
        let fixture = Self {
            client,
            state,
            server,
        };
        fixture.reply(
            "/v1/account/me",
            json!({"data":{"walletAddress":"0xfixture"}}),
        );
        fixture.reply(
            "/v1/image/downloadUrl",
            json!({"data":{"downloadUrl":"https://example.test/image.png"}}),
        );
        fixture.reply(
            "/v1/media/downloadUrl",
            json!({"data":{"downloadUrl":"https://example.test/media.mp4"}}),
        );
        fixture
    }
    fn reply(&self, path: &str, value: Value) {
        self.state
            .replies
            .lock()
            .insert(path.into(), (StatusCode::OK, value));
    }
    fn error(&self, path: &str, status: StatusCode, message: &str) {
        self.state
            .replies
            .lock()
            .insert(path.into(), (status, json!({"message":message})));
    }
    fn calls(&self, path: &str) -> Vec<BTreeMap<String, String>> {
        self.state
            .calls
            .lock()
            .iter()
            .filter(|(name, _)| name == path)
            .map(|(_, query)| query.clone())
            .collect()
    }
    fn record(&self, model: Value, jobs: Value) {
        self.reply("/v2/projects/P",json!({"data":{"project":{"id":"P","status":"completed","finished":true,"model":model,"workerJobs":[],"completedWorkerJobs":jobs}}}));
    }
    fn track(&self, kind: &str, model: &str, format: Option<&str>) -> Project {
        let project = Project::new(
            "P".into(),
            json!({"type":kind,"modelId":model,"numberOfMedia":1,"outputFormat":format}),
            false,
            Arc::downgrade(&self.client.projects.inner),
        );
        self.client
            .projects
            .inner
            .projects
            .write()
            .insert("P".into(), project.clone());
        project
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn serve(State(state): State<Arc<FixtureState>>, request: Request<Body>) -> Response {
    let path = request.uri().path().to_owned();
    let query = url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
        .into_owned()
        .collect();
    state.calls.lock().push((path.clone(), query));
    let held = {
        let mut hold = state.hold.lock();
        if hold.as_deref() == Some(&path) {
            hold.take();
            true
        } else {
            false
        }
    };
    if held {
        state.entered.notify_one();
        state.release.notified().await;
    }
    let (status, body) = state
        .replies
        .lock()
        .get(&path)
        .cloned()
        .unwrap_or((StatusCode::NOT_FOUND, json!({"message":"not found"})));
    (status, Json(body)).into_response()
}
fn completed(id: &str) -> Value {
    json!({"imgID":id,"status":"jobCompleted"})
}

#[tokio::test]
async fn results_are_owner_scoped_deduplicated_and_preserve_individual_outcomes() {
    let fixture = Fixture::start().await;
    fixture.reply("/v2/projects/P",json!({"data":{"project":{"id":"P","status":"completed","finished":true,"model":{"id":"minimax-h3-ref2va-fp8_r2v","type":"video"},"completedWorkerJobs":[
        {"id":"row0","imgID":"GOOD","status":"jobCompleted","seedUsed":0},
        {"imgID":"WITHHELD","status":"jobCompleted","triggeredNSFWFilter":true},
        {"imgID":"FAILED","status":"jobError","reason":"genfailure"},
        {"imgID":"CANCELED","status":"jobError","reason":"artistCanceled"},
        {"imgID":"DIRECT","status":"jobCompleted","resultUrl":"https://vendor.test/out.mp4","triggeredNSFWFilter":true,"nsfwDetected":true}
    ],"workerJobs":[{"imgID":"GOOD","status":"jobStarted"}]}}}));
    let result = fixture.client.projects.get_result("p", None).await.unwrap();
    assert!(result.finished);
    assert_eq!(result.jobs.len(), 5);
    assert_eq!(result.jobs[0].seed, Some(0));
    assert_eq!(result.jobs[0].kind, Some(ResultMediaKind::Video));
    assert_eq!(
        result.jobs[1].url_unavailable,
        Some(ResultUrlUnavailable::SensitiveContent)
    );
    assert_eq!(result.jobs[2].status, "failed");
    assert_eq!(result.jobs[2].reason.as_deref(), Some("genfailure"));
    assert_eq!(result.jobs[3].status, "canceled");
    assert_eq!(
        result.jobs[4].url.as_deref(),
        Some("https://vendor.test/out.mp4")
    );
    assert!(fixture.calls("/v1/image/downloadUrl").is_empty());
    assert_eq!(fixture.calls("/v1/media/downloadUrl")[0]["jobId"], "P");
    assert_eq!(fixture.client.projects.tracked_projects().len(), 0);
    assert!(
        matches!(fixture.client.projects.get_result("MISSING",None).await,Err(Error::Api(error)) if error.status==404)
    );
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn result_kind_hint_is_last_fallback_and_signing_failure_is_per_result() {
    let fixture = Fixture::start().await;
    fixture.record(json!({"id":"future-model"}),json!([completed("UNKNOWN"),{"imgID":"STORED","status":"jobCompleted","resultUrl":"https://vendor.test/out"}]));
    let unknown = fixture.client.projects.get_result("P", None).await.unwrap();
    assert_eq!(
        unknown.jobs[0].url_unavailable,
        Some(ResultUrlUnavailable::UnknownMediaKind)
    );
    assert!(fixture.calls("/v1/image/downloadUrl").is_empty());
    fixture.error(
        "/v1/image/downloadUrl",
        StatusCode::SERVICE_UNAVAILABLE,
        "try later",
    );
    let result = fixture
        .client
        .projects
        .get_result(
            "P",
            Some(GetProjectResultOptions {
                kind: Some(ResultMediaKind::Image),
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        result.jobs[0].url_unavailable,
        Some(ResultUrlUnavailable::DownloadUrlFailed)
    );
    assert_eq!(
        result.jobs[1].url.as_deref(),
        Some("https://vendor.test/out")
    );
    assert!(fixture.calls("/v1/media/downloadUrl").is_empty());
    fixture.record(json!({"id":"future-model"}),json!([{"imgID":"AUDIO","status":"jobCompleted","result":{"artifacts":[{"contentType":"audio/wav"}]}}]));
    let result = fixture
        .client
        .projects
        .get_result(
            "P",
            Some(GetProjectResultOptions {
                kind: Some(ResultMediaKind::Image),
            }),
        )
        .await
        .unwrap();
    assert_eq!(result.jobs[0].kind, Some(ResultMediaKind::Audio));
    assert_eq!(
        fixture.calls("/v1/media/downloadUrl")[0]["contentType"],
        "audio/wav"
    );
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn stored_result_null_fields_use_nested_evidence_without_guessing_malformed_kinds() {
    let fixture = Fixture::start().await;
    fixture.record(json!({"id":"future-model"}), json!([
        {"imgID":"AUDIO","status":"jobCompleted","outputFormat":null,
            "result":{"outputFormat":"wav"}},
        {"imgID":"VIDEO","status":"jobCompleted","artifacts":null,
            "result":{"artifacts":[{"contentType":"video/mp4","success":true}]}},
        {"imgID":"MALFORMED-FORMAT","status":"jobCompleted","outputFormat":7,
            "result":{"outputFormat":"wav"}},
        {"imgID":"MALFORMED-ARTIFACTS","status":"jobCompleted","artifacts":{"contentType":"image/png"}}
    ]));
    let result = fixture.client.projects.get_result("P", None).await.unwrap();
    assert_eq!(result.jobs[0].kind, Some(ResultMediaKind::Audio));
    assert_eq!(result.jobs[1].kind, Some(ResultMediaKind::Video));
    for job in &result.jobs[2..] {
        assert_eq!(job.kind, None);
        assert_eq!(
            job.url_unavailable,
            Some(ResultUrlUnavailable::UnknownMediaKind)
        );
    }
    assert!(fixture.calls("/v1/image/downloadUrl").is_empty());
    let calls = fixture.calls("/v1/media/downloadUrl");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["id"], "AUDIO");
    assert_eq!(calls[0]["contentType"], "audio/wav");
    assert_eq!(calls[1]["id"], "VIDEO");
    assert!(!calls[1].contains_key("contentType"));
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn narrow_media_refusal_is_remembered_even_when_first_media_request_fails() {
    let fixture = Fixture::start().await;
    fixture.error(
        "/v1/image/downloadUrl",
        StatusCode::NOT_FOUND,
        "This result is media, not an image; request it from /v1/media/downloadUrl",
    );
    fixture.error(
        "/v1/media/downloadUrl",
        StatusCode::INTERNAL_SERVER_ERROR,
        "retry later",
    );
    let project = fixture.track("image", "future-model", None);
    handle_job_result(
        &fixture.client.projects.inner,
        &json!({"jobID":"P","imgID":"J"}),
    )
    .await;
    let job = project.job("J").unwrap();
    assert!(job.result_url().is_none());
    assert_eq!(fixture.calls("/v1/image/downloadUrl").len(), 1);
    assert_eq!(fixture.calls("/v1/media/downloadUrl").len(), 1);
    fixture.reply(
        "/v1/media/downloadUrl",
        json!({"data":{"downloadUrl":"https://example.test/result.mp4"}}),
    );
    assert_eq!(
        job.get_result_url().await.unwrap(),
        "https://example.test/result.mp4"
    );
    assert_eq!(fixture.calls("/v1/image/downloadUrl").len(), 1);
    assert_eq!(fixture.calls("/v1/media/downloadUrl").len(), 2);
    for (status, message) in [
        (StatusCode::NOT_FOUND, "Download not found"),
        (StatusCode::FORBIDDEN, "/v1/media/downloadUrl"),
        (StatusCode::INTERNAL_SERVER_ERROR, "/v1/media/downloadUrl"),
    ] {
        fixture.error("/v1/image/downloadUrl", status, message);
        fixture.record(json!({"type":"image"}), json!([completed("OTHER")]));
        let result = fixture.client.projects.get_result("P", None).await.unwrap();
        assert_eq!(
            result.jobs[0].url_unavailable,
            Some(ResultUrlUnavailable::DownloadUrlFailed)
        );
        assert_eq!(fixture.calls("/v1/media/downloadUrl").len(), 2);
    }
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn untracked_results_use_frame_evidence_without_guessing_an_image() {
    let fixture = Fixture::start().await;
    let mut events = fixture.client.projects.subscribe();
    for frame in [
        json!({}),
        json!({"artifacts":[{"contentType":"video/mp4","success":false}]}),
    ] {
        let mut frame = frame;
        frame["jobID"] = json!("OTHER");
        frame["imgID"] = json!("J");
        handle_job_result(&fixture.client.projects.inner, &frame).await;
        assert!(events.try_recv().unwrap().data["resultUrl"].is_null());
    }
    assert!(fixture.state.calls.lock().is_empty());
    let cases = [
        (
            json!({"artifacts":[{"contentType":"image/png"},{"contentType":"video/mp4"}]}),
            "/v1/media/downloadUrl",
            None,
        ),
        (
            json!({"outputFormat":"wav"}),
            "/v1/media/downloadUrl",
            Some("audio/wav"),
        ),
        (
            json!({"artifacts":[{"contentType":"image/jpeg"}]}),
            "/v1/image/downloadUrl",
            None,
        ),
        (
            json!({"artifacts":[{"contentType":"image/png"},{"contentType":"model/gltf-binary"}]}),
            "/v1/media/downloadUrl",
            Some("model/gltf-binary"),
        ),
    ];
    for (mut frame, path, content_type) in cases {
        frame["jobID"] = json!("OTHER");
        frame["imgID"] = json!("J");
        handle_job_result(&fixture.client.projects.inner, &frame).await;
        assert!(events.try_recv().unwrap().data["resultUrl"].is_string());
        let calls = fixture.calls(path);
        assert_eq!(
            calls.last().unwrap().get("contentType").map(String::as_str),
            content_type
        );
    }
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn catalog_absence_known_models_project_types_and_pixal_override_select_correct_endpoint() {
    let fixture = Fixture::start().await;
    for (requested, model, catalog, expected) in [
        (
            "image",
            "wan_v2.2-14b-fp8_s2v",
            json!({"id":"wan_v2.2-14b-fp8_s2v"}),
            ResultMediaKind::Video,
        ),
        (
            "image",
            "qwen3_tts_1.7b_custom_voice_bf16",
            json!({"id":"qwen3_tts_1.7b_custom_voice_bf16","media":"future"}),
            ResultMediaKind::Audio,
        ),
        (
            "video",
            "future-model",
            json!({"id":"future-model"}),
            ResultMediaKind::Video,
        ),
        (
            "image",
            "pixal3d_int8_i23d",
            json!({"id":"pixal3d_int8_i23d","media":"image"}),
            ResultMediaKind::Model,
        ),
    ] {
        *fixture.client.projects.inner.supported_models.write() = Some(api::TimedValue {
            value: json!([catalog]),
            loaded_at: Instant::now(),
        });
        let project = fixture.track(requested, model, None);
        let job = project.ensure_job("J");
        assert_eq!(job.media_type(), expected.as_str());
        handle_job_result(
            &fixture.client.projects.inner,
            &json!({"jobID":"P","imgID":"J"}),
        )
        .await;
        assert!(job.result_url().is_some());
    }
    assert!(fixture.calls("/v1/image/downloadUrl").is_empty());
    assert_eq!(fixture.calls("/v1/media/downloadUrl").len(), 4);
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn tracked_video_signing_ignores_incompatible_artifact_mime() {
    let fixture = Fixture::start().await;
    for (format, artifacts, content_type) in [
        (None, json!([{"contentType":"image/png"}]), None),
        (
            None,
            json!([{"contentType":"video/mp4","success":false},{"contentType":"image/png"}]),
            None,
        ),
        (
            Some("webm"),
            json!([{"contentType":"image/png"}]),
            Some("video/webm"),
        ),
    ] {
        let project = fixture.track("video", "minimax-h3-ref2va-fp8_r2v", format);
        handle_job_result(
            &fixture.client.projects.inner,
            &json!({"jobID":"P","imgID":"J","artifacts":artifacts}),
        )
        .await;
        assert!(project.job("J").unwrap().result_url().is_some());
        let calls = fixture.calls("/v1/media/downloadUrl");
        assert_eq!(
            calls.last().unwrap().get("contentType").map(String::as_str),
            content_type
        );
    }
    assert!(fixture.calls("/v1/image/downloadUrl").is_empty());
    assert_eq!(fixture.calls("/v1/media/downloadUrl").len(), 3);
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn recent_history_is_bounded_grouped_sorted_and_account_specific() {
    let fixture = Fixture::start().await;
    let now = Utc::now().timestamp_millis();
    fixture.reply("/v1/jobs/list",json!({"data":{"jobs":[
        {"imgID":"A0","status":"jobCompleted","endTime":now-5000,"parentRequest":{"id":"A","model":{"id":"video","name":"Video"},"appSource":"app"}},
        {"id":"B0","status":"jobCompleted","endTime":now-1000,"parentRequest":{"id":"B"}},
        {"imgID":"A1","status":"jobCompleted","endTime":now-3000,"triggeredNSFWFilter":true,"parentRequest":{"id":"A"}},
        {"id":"orphan","status":"jobCompleted","endTime":now}
    ]}}));
    let projects = fixture
        .client
        .projects
        .list_recent(Some(ListRecentProjectsOptions {
            since: Some(0),
            limit: Some(500),
            app_source: Some("app".into()),
        }))
        .await
        .unwrap();
    assert_eq!(
        projects
            .iter()
            .map(|project| project.id.as_str())
            .collect::<Vec<_>>(),
        vec!["B", "A"]
    );
    assert_eq!(projects[1].jobs.len(), 2);
    assert!(projects[1].jobs[1].sensitive_content_withheld);
    assert_eq!(projects[1].finished_at, Some(now - 3000));
    let calls = fixture.calls("/v1/jobs/list");
    let query = &calls[0];
    assert_eq!(query["address"], "0xfixture");
    assert_eq!(query["role"], "artist");
    assert_eq!(query["state"], "completed");
    assert_eq!(query["mediaOnly"], "true");
    assert_eq!(query["limit"], "100");
    assert_eq!(query["appSource"], "app");
    assert!(now - query["since"].parse::<i64>().unwrap() < 7 * 24 * 3600 * 1000);
    fixture.client.projects.list_recent(None).await.unwrap();
    let calls = fixture.calls("/v1/jobs/list");
    let query = &calls[1];
    assert_eq!(query["limit"], "50");
    assert!(!query.contains_key("appSource"));
    assert!((now - 24 * 3600 * 1000 - query["since"].parse::<i64>().unwrap()).abs() < 5000);
    fixture.client.projects.inner.client.clear_auth();
    assert!(fixture.client.projects.list_recent(None).await.is_err());
    assert_eq!(fixture.calls("/v1/jobs/list").len(), 2);
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn session_end_invalidates_delayed_results_and_old_job_handles() {
    for path in ["/v2/projects/P", "/v1/media/downloadUrl", "/v1/jobs/list"] {
        let fixture = Fixture::start().await;
        fixture.record(json!({"type":"video"}), json!([completed("J")]));
        fixture.reply("/v1/jobs/list", json!({"data":{"jobs":[]}}));
        let project = fixture.track("video", "future-video", None);
        let job = project.ensure_job("J");
        job.update(|state| state.status = JobStatus::Completed, &["status"]);
        *fixture.state.hold.lock() = Some(path.into());
        let api = fixture.client.projects.clone();
        let task = tokio::spawn(async move {
            if path == "/v1/jobs/list" {
                api.list_recent(None).await.map(|_| ())
            } else {
                api.get_result("P", None).await.map(|_| ())
            }
        });
        tokio::time::timeout(Duration::from_secs(2), fixture.state.entered.notified())
            .await
            .unwrap();
        fixture.client.projects.inner.client.clear_auth();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        fixture.state.release.notify_one();
        assert!(job.get_result_url().await.is_err());
        fixture.client.close().await.unwrap();
    }
}
