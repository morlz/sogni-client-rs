use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    routing::{get, put},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use super::*;
use crate::{SogniClient, projects::api::TimedValue};

const ID: &str = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
const WATCHDOG: Duration = Duration::from_secs(10);

struct Upload {
    url: Url,
    entered: Notify,
    release: Notify,
    status_calls: AtomicUsize,
}

async fn socket_fixture() -> (
    Url,
    mpsc::UnboundedReceiver<Value>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let Ok(mut socket) = accept_async(stream).await else {
                    return;
                };
                socket
                    .send(Message::Text(
                        json!({"type":"authenticated","data":STANDARD.encode("{}")})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    match message {
                        Message::Text(text) => {
                            let envelope: Value = serde_json::from_str(&text).unwrap();
                            if envelope["type"] == "jobRequest" {
                                let bytes =
                                    STANDARD.decode(envelope["data"].as_str().unwrap()).unwrap();
                                tx.send(serde_json::from_slice(&bytes).unwrap()).unwrap();
                            }
                        }
                        Message::Ping(data) => {
                            if socket.send(Message::Pong(data)).await.is_err() {
                                break;
                            }
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            });
        }
    });
    (url, rx, task)
}

#[tokio::test]
async fn upload_preparation_cannot_consume_a_new_projects_admission_grace() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint: Url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let upload = Arc::new(Upload {
        url: endpoint.join("upload").unwrap(),
        entered: Notify::new(),
        release: Notify::new(),
        status_calls: AtomicUsize::new(0),
    });
    let router = Router::new()
        .route(
            "/v1/account/me",
            get(|| async { Json(json!({"data":{"username":"fixture"}})) }),
        )
        .route(
            "/v1/assets/capabilities",
            get(|| async { Json(json!({"data":{"enabled":false}})) }),
        )
        .route(
            "/v1/image/uploadUrl",
            get(|State(upload): State<Arc<Upload>>| async move {
                Json(json!({"data":{"uploadUrl":upload.url}}))
            }),
        )
        .route(
            "/upload",
            put(
                |State(upload): State<Arc<Upload>>, bytes: Bytes| async move {
                    assert_eq!(bytes.as_ref(), b"\x89PNGfixture-content");
                    upload.entered.notify_one();
                    upload.release.notified().await;
                    Json(json!({}))
                },
            ),
        )
        .route(
            "/v2/projects/{id}",
            get(|State(upload): State<Arc<Upload>>| async move {
                upload.status_calls.fetch_add(1, Ordering::SeqCst);
                Json(json!({"data":{"project":{
                    "id":ID,"status":"failed","finished":true,"statusOnly":true,
                    "workerJobs":[],"completedWorkerJobs":[],"reason":"fixture finished"
                }}}))
            }),
        )
        .with_state(upload.clone());
    let http = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (socket_url, mut sent, socket) = socket_fixture().await;
    let client = SogniClient::builder()
        .app_id("local-preparation-grace-fixture")
        .api_key("local-fixture")
        .rest_endpoint(endpoint)
        .socket_endpoint(socket_url)
        .defer_socket_start(true)
        .request_timeout(WATCHDOG)
        .build()
        .await
        .unwrap();
    // Seed the normal caches so socket traffic can use its own loopback server.
    *client.projects.inner.supported_models.write() = Some(TimedValue {
        value: json!([{"id":"fixture-model","tier":"fixture"}]),
        loaded_at: Instant::now(),
    });
    *client.projects.inner.model_tiers.write() = Some(TimedValue {
        value: json!({"fixture":{"type":"image"}}),
        loaded_at: Instant::now(),
    });
    let creating = client.projects.clone();
    let pending = tokio::spawn(async move {
        creating
            .create_with_id_detailed(
                ID,
                ProjectRequest::image("fixture-model", "A fixture lighthouse").asset(
                    AssetRole::StartingImage,
                    MediaSource::bytes(b"\x89PNGfixture-content".as_slice()),
                ),
            )
            .await
    });
    tokio::time::timeout(WATCHDOG, upload.entered.notified())
        .await
        .unwrap();
    assert!(!pending.is_finished());
    assert!(client.projects.tracked_projects().is_empty());
    let session = client.projects.inner.client.rest.request_session();
    let empty_snapshot = json!({"activeProjects":[],"unclaimedCompletedProjects":[]});
    // An absent server snapshot remains harmless even when its observation is
    // well past the grace that would have begun before this held upload.
    let preparing = client
        .projects
        .reconcile(
            empty_snapshot.clone(),
            "upload-in-flight",
            Utc::now() + chrono::Duration::minutes(1),
            &session,
            &HashMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(preparing["lost"], json!([]));
    assert!(client.projects.tracked_projects().is_empty());
    assert_eq!(upload.status_calls.load(Ordering::SeqCst), 0);

    let released_at = Utc::now();
    upload.release.notify_one();
    client.projects.inner.client.start().await.unwrap();
    let project = tokio::time::timeout(WATCHDOG, pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let request = tokio::time::timeout(WATCHDOG, sent.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request["jobID"], ID);
    let started_at = project.snapshot().started_at;
    assert!(started_at >= released_at);
    assert!(started_at <= Utc::now());
    assert_eq!(project.status(), ProjectStatus::Pending);

    let grace = chrono::Duration::from_std(RECENTLY_CREATED_GRACE).unwrap();
    let within_grace = client
        .projects
        .reconcile(
            empty_snapshot.clone(),
            "fresh-submission",
            started_at + grace - chrono::Duration::milliseconds(1),
            &session,
            &HashMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(within_grace["lost"], json!([]));
    assert_eq!(project.status(), ProjectStatus::Pending);
    assert_eq!(upload.status_calls.load(Ordering::SeqCst), 0);
    let after_grace = tokio::time::timeout(
        WATCHDOG,
        client.projects.reconcile(
            empty_snapshot,
            "admission-grace-expired",
            started_at + grace + chrono::Duration::milliseconds(1),
            &session,
            &HashMap::new(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(upload.status_calls.load(Ordering::SeqCst), 1);
    assert_eq!(project.status(), ProjectStatus::Failed);
    assert_eq!(after_grace["completed"], json!([ID]));
    assert_eq!(after_grace["lost"], json!([]));
    client.close().await.unwrap();
    http.abort();
    socket.abort();
}
