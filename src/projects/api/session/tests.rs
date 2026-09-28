use super::*;
use crate::{
    AuthKind, ClientConfig, MinimaxH3Keyframe,
    auth::AuthManager,
    transport::{ApiClient, HttpClients},
};
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, put},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

const WATCHDOG: Duration = Duration::from_secs(5);

fn api(endpoint: Url) -> (ProjectsApi, AuthManager) {
    let http = HttpClients::build(Duration::from_secs(30)).unwrap();
    let auth = AuthManager::new(
        AuthKind::ApiKey,
        endpoint.clone(),
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("A").unwrap();
    let mut socket = endpoint.clone();
    socket.set_scheme("ws").unwrap();
    let client = Arc::new(
        ApiClient::new(
            ClientConfig {
                app_id: "session-fixture".into(),
                rest_endpoint: endpoint,
                socket_endpoint: socket,
                auth_kind: AuthKind::ApiKey,
                ..Default::default()
            },
            auth.clone(),
            http,
        )
        .unwrap(),
    );
    (ProjectsApi::new(client), auth)
}

async fn server(router: Router) -> (Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (endpoint, task)
}

#[derive(Clone)]
struct Uploads {
    endpoint: Url,
    registrations: Arc<AtomicUsize>,
    transfers: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[tokio::test]
async fn switching_during_first_keyframe_upload_prevents_later_uploads_and_submission() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint: Url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let state = Uploads {
        endpoint: endpoint.clone(),
        registrations: Arc::new(AtomicUsize::new(0)),
        transfers: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
    };
    let app = Router::new()
        .route("/api/v1/models/list", get(|| async { Json(json!([{"id":"minimax-h3-fl2va-fp8_i2v","tier":"h3","media":"video"}])) }))
        .route("/api/v2/models/tiers", get(|| async { Json(json!({"h3":{"type":"video","steps":{"min":20,"max":20,"default":20},"guidance":{"min":1,"max":1,"default":1}}})) }))
        .route("/v1/assets/capabilities", get(|| async { Json(json!({"data":{"enabled":false}})) }))
        .route("/v1/image/uploadUrl", get(|State(state): State<Uploads>| async move {
            state.registrations.fetch_add(1,Ordering::SeqCst);
            Json(json!({"data":{"uploadUrl":state.endpoint.join("upload").unwrap()}}))
        }))
        .route("/upload", put(|State(state): State<Uploads>| async move {
            state.transfers.fetch_add(1,Ordering::SeqCst);
            state.entered.notify_one(); state.release.notified().await;
            Json(json!({}))
        })).with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (api, auth) = api(endpoint);
    let submitting = api.clone();
    let mut pending = tokio::spawn(async move {
        submitting
            .create_with_id_detailed(
                "00000000-0000-4000-8000-000000000001",
                ProjectRequest::video("minimax-h3-fl2va-fp8_i2v", "Fixture keyframes")
                    .param("frames", 243)
                    .param("referenceImage", true)
                    .steps(20)
                    .guidance(1.0)
                    .keyframes(vec![
                        MinimaxH3Keyframe {
                            image: MediaSource::named_bytes("first", "one.png", "image/png"),
                            frame_index: 30,
                        },
                        MinimaxH3Keyframe {
                            image: MediaSource::named_bytes("second", "two.png", "image/png"),
                            frame_index: 90,
                        },
                    ]),
            )
            .await
    });
    tokio::time::timeout(WATCHDOG, async {
        tokio::select! {
            () = state.entered.notified() => {},
            result = &mut pending => panic!("upload preparation ended before transfer: {}", result.unwrap().unwrap_err().cause()),
        }
    }).await.unwrap();
    auth.authenticate_api_key("B").unwrap();
    let error = tokio::time::timeout(WATCHDOG, pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.phase(), SubmissionPhase::AssetUpload);
    assert!(
        error
            .cause()
            .to_string()
            .contains("account session changed")
    );
    assert_eq!(state.registrations.load(Ordering::SeqCst), 1);
    assert_eq!(state.transfers.load(Ordering::SeqCst), 1);
    assert!(api.tracked_projects().is_empty());
    state.release.notify_one();
    api.inner.client.close().await.unwrap();
    server.abort();
}

fn track(api: &ProjectsApi, id: &str) -> Project {
    let project = Project::new(
        id.into(),
        json!({"type":"image","numberOfMedia":1}),
        false,
        Arc::downgrade(&api.inner),
    );
    api.inner
        .projects
        .write()
        .insert(id.into(), project.clone());
    project
}

#[tokio::test]
async fn delayed_failed_send_cleanup_preserves_reused_uuid_and_new_submission() {
    let (api, auth) = api("http://127.0.0.1:9/".parse().unwrap());
    let old = track(&api, "RESERVED_UUID");
    api.inner
        .submission
        .lock()
        .unadmitted
        .insert(old.id(), json!({"owner":"A"}));
    auth.authenticate_api_key("B").unwrap();
    api.clear_previous_sessions();
    let new = track(&api, "RESERVED_UUID");
    api.inner
        .submission
        .lock()
        .unadmitted
        .insert(new.id(), json!({"owner":"B"}));
    // Resume A's failed-send branch only after B has reused the reserved UUID.
    api.remove_failed_submission(&old);
    assert!(api.inner.projects.read()["RESERVED_UUID"].same_handle(&new));
    assert_eq!(
        api.inner.submission.lock().unadmitted["RESERVED_UUID"]["owner"],
        "B"
    );
    assert!(new.check_session().is_ok());
    // Its own later failed send still retires B's unpublished request normally.
    api.remove_failed_submission(&new);
    assert!(api.inner.projects.read().is_empty());
    assert!(api.inner.submission.lock().unadmitted.is_empty());
    api.inner.client.close().await.unwrap();
}

#[tokio::test]
async fn session_end_retires_recovery_and_old_handles_without_canceling_remote_work() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (endpoint, server) = server(Router::new().fallback(move || {
        let counted = counted.clone();
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            Json(json!({}))
        }
    }))
    .await;
    let (api, auth) = api(endpoint);
    let old = track(&api, "OLD");
    let job = old.ensure_job("CHILD");
    {
        let mut state = api.inner.submission.lock();
        state
            .unadmitted
            .insert("OLD".into(), json!({"jobID":"OLD"}));
        state.awaiting.insert("OLD".into());
        state.sent_on.insert("OLD".into(), (1, old.auth_session()));
        state.submitted_at.insert("OLD".into(), Utc::now());
    }
    api.inner
        .recovered_completed_ids
        .write()
        .insert("OLD_COMPLETED".into());
    api.schedule_recheck(Duration::from_millis(25));
    let waiting = old.clone();
    let waiter = tokio::spawn(async move { waiting.wait_for_completion(None).await });
    auth.authenticate_api_key("B").unwrap();
    let new = track(&api, "NEW");
    api.clear_previous_sessions();
    let error = tokio::time::timeout(WATCHDOG, waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error.to_string().contains("may still be running"),
        "{error}"
    );
    assert!(old.cancel().await.is_err());
    assert!(job.get_result_url().await.is_err());
    assert!(new.check_session().is_ok());
    assert_eq!(api.tracked_projects().len(), 1);
    {
        let state = api.inner.submission.lock();
        assert!(
            state.unadmitted.is_empty()
                && state.awaiting.is_empty()
                && state.sent_on.is_empty()
                && state.submitted_at.is_empty()
        );
    }
    assert!(api.inner.recovered_completed_ids.read().is_empty());
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    api.inner.client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn stale_sync_waiting_for_reconciliation_lock_never_rehydrates_an_old_account() {
    let entered = Arc::new(Notify::new());
    let observed = entered.clone();
    let (endpoint, server) = server(Router::new().route(
        "/api/v1/artist/projects/sync",
        get(move |headers: HeaderMap| {
            let observed = observed.clone();
            async move {
                observed.notify_one();
                let projects = if headers.get("api-key").unwrap() == "A" {
                    json!([{"id":"OLD","status":"queued","workerJobs":[],"completedWorkerJobs":[]}])
                } else {
                    json!([])
                };
                Json(json!({"activeProjects":projects,"unclaimedCompletedProjects":[]}))
            }
        }),
    ))
    .await;
    let (api, auth) = api(endpoint);
    let lock = api.inner.sync_lock.lock().await;
    let syncing = api.clone();
    let pending = tokio::spawn(async move { syncing.sync("old-request").await });
    tokio::time::timeout(WATCHDOG, entered.notified())
        .await
        .unwrap();
    auth.authenticate_api_key("B").unwrap();
    assert!(
        tokio::time::timeout(WATCHDOG, pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    drop(lock);
    api.sync("new-request").await.unwrap();
    assert!(api.tracked_projects().is_empty());
    api.inner.client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn nested_project_recovery_preserves_its_matching_unauthorized_status() {
    let (endpoint, server) = server(Router::new().fallback(|| async {
        (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(json!({"message":"session expired"})),
        )
    }))
    .await;
    let (api, auth) = api(endpoint);
    let error = api.recover_project("ORIGINAL").await.unwrap_err();
    assert!(matches!(error,Error::Api(error) if error.status == 401));
    assert!(!auth.is_authenticated());
    api.inner.client.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn a_refreshed_different_wallet_retires_existing_project_and_job_handles() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    fn token(address: &str, expired: bool) -> String {
        format!(
            "e30.{}.fixture",
            URL_SAFE_NO_PAD.encode(
                json!({"addr":address,"exp":if expired {1} else {4_102_444_800_u64}}).to_string()
            )
        )
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (endpoint, server) = server(Router::new()
        .route("/v1/account/refresh-token",axum::routing::post(|| async {
            Json(json!({"data":{"token":token("B",false),"refreshToken":token("refresh",false)}}))
        }))
        .fallback(move || {let counted=counted.clone();async move { counted.fetch_add(1,Ordering::SeqCst);Json(json!({})) }})).await;
    let http = HttpClients::build(WATCHDOG).unwrap();
    let auth = AuthManager::new(
        AuthKind::Token,
        endpoint.clone(),
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_tokens(token("A", false), token("refresh", false))
        .await
        .unwrap();
    let client = Arc::new(
        ApiClient::new(
            ClientConfig {
                rest_endpoint: endpoint,
                disable_socket: true,
                ..Default::default()
            },
            auth.clone(),
            http,
        )
        .unwrap(),
    );
    let api = ProjectsApi::new(client);
    let project = track(&api, "A_PROJECT");
    let job = project.ensure_job("A_JOB");
    auth.authenticate_tokens(token("A", true), token("refresh", false))
        .await
        .unwrap();
    assert!(project.cancel().await.is_err());
    assert!(job.get_result_url().await.is_err());
    assert!(
        project
            .wait_for_completion(None)
            .await
            .unwrap_err()
            .to_string()
            .contains("may still be running")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    api.inner.client.close().await.unwrap();
    server.abort();
}
