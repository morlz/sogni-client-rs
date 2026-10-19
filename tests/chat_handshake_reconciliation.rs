use std::{task::Poll, time::Duration};

use axum::{Json, Router};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt, future::join_all};
use serde_json::{Value, json};
use sogni_client::{ChatStream, Error, SogniClient, is_retryable_chat_error};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

const WATCHDOG: Duration = Duration::from_secs(10);

struct Peer {
    frames: mpsc::UnboundedSender<(String, Value)>,
    requests: mpsc::UnboundedReceiver<Value>,
    socket_task: JoinHandle<()>,
    rest_task: JoinHandle<()>,
    barrier_number: u64,
}

impl Peer {
    fn frame(&self, name: &str, data: Value) {
        self.frames.send((name.to_owned(), data)).unwrap();
    }

    async fn request(&mut self) -> Value {
        tokio::time::timeout(WATCHDOG, self.requests.recv())
            .await
            .expect("chat request must reach the fixture")
            .unwrap()
    }

    async fn barrier(&mut self, client: &SogniClient) {
        self.barrier_number += 1;
        let key = format!("fixture-barrier-{}", self.barrier_number);
        let mut events = client.chat.subscribe();
        let mut models = serde_json::Map::new();
        models.insert(key.clone(), json!({"workers": 1}));
        self.frame("swarmLLMModels", Value::Object(models));
        tokio::time::timeout(WATCHDOG, async {
            loop {
                let event = events.recv().await.unwrap();
                if event.name == "modelsUpdated" && event.data.get(&key).is_some() {
                    return;
                }
            }
        })
        .await
        .expect("chat listener must consume all preceding fixture frames");
    }

    async fn finish(self, client: SogniClient) {
        client.close().await.unwrap();
        self.socket_task.abort();
        self.rest_task.abort();
    }
}

async fn fixture() -> (SogniClient, Peer) {
    let rest = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rest_endpoint = format!("http://{}/", rest.local_addr().unwrap());
    let rest_task = tokio::spawn(async move {
        axum::serve(
            rest,
            Router::new().fallback(|| async {
                Json(json!({"data": {}, "activeProjects": [], "unclaimedCompletedProjects": []}))
            }),
        )
        .await
        .unwrap();
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socket_endpoint = format!("ws://{}/", listener.local_addr().unwrap());
    let (frames, mut incoming_frames) = mpsc::unbounded_channel::<(String, Value)>();
    let (requests, received_requests) = mpsc::unbounded_channel();
    let socket_task = tokio::spawn(async move {
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        loop {
            tokio::select! {
                frame = incoming_frames.recv() => {
                    let Some((name, data)) = frame else { return; };
                    let envelope = json!({"type": name, "data": STANDARD.encode(data.to_string())});
                    if socket.send(Message::Text(envelope.to_string().into())).await.is_err() {
                        return;
                    }
                }
                message = socket.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            let envelope: Value = serde_json::from_str(&text).unwrap();
                            assert_eq!(envelope["type"], "llmJobRequest");
                            let payload = STANDARD.decode(envelope["data"].as_str().unwrap()).unwrap();
                            requests.send(serde_json::from_slice::<Value>(&payload).unwrap()).unwrap();
                        }
                        Some(Ok(Message::Ping(bytes))) => {
                            if socket.send(Message::Pong(bytes)).await.is_err() { return; }
                        }
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                        _ => {}
                    }
                }
            }
        }
    });
    let client = SogniClient::builder()
        .app_id("chat-handshake-fixture")
        .api_key("fixture-key")
        .rest_endpoint(rest_endpoint.parse().unwrap())
        .socket_endpoint(socket_endpoint.parse().unwrap())
        .request_timeout(WATCHDOG)
        .build()
        .await
        .unwrap();
    (
        client,
        Peer {
            frames,
            requests: received_requests,
            socket_task,
            rest_task,
            barrier_number: 0,
        },
    )
}

fn params() -> Value {
    json!({"model": "fixture", "messages": [{"role": "user", "content": "hello"}]})
}

async fn open(client: &SogniClient) -> ChatStream {
    tokio::time::timeout(WATCHDOG, client.chat.stream_completion(&params()))
        .await
        .expect("socket submission must finish")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handshake_without_prior_loss_settles_missing_and_keeps_listed_streams() {
    let (client, mut peer) = fixture().await;
    peer.frame("authenticated", json!({"activeLLMJobIDs": []}));
    peer.barrier(&client).await;
    let mut missing = open(&client).await;
    let missing_request = peer.request().await;
    assert_eq!(missing_request["jobID"], missing.job_id());
    let mut kept = open(&client).await;
    let kept_request = peer.request().await;
    assert_eq!(kept_request["jobID"], kept.job_id());
    let mut events = client.chat.subscribe();
    let mut transport_events = client.subscribe();

    // No reconnect or connecting event occurs between these sent requests and
    // this new authenticated snapshot on the same native connection.
    peer.frame(
        "authenticated",
        json!({"activeLLMJobIDs": [kept.job_id().to_lowercase()]}),
    );
    peer.barrier(&client).await;
    let error = tokio::time::timeout(WATCHDOG, missing.wait(None))
        .await
        .expect("missing sent work must settle without a grace timeout")
        .unwrap_err();
    assert!(
        matches!(&error, Error::Chat(chat) if chat.error_type.as_deref() == Some("transport_lost"))
    );
    assert!(is_retryable_chat_error(&error));
    assert!(is_retryable_chat_error(
        &missing.next().await.unwrap().unwrap_err()
    ));
    assert!(missing.next().await.is_none());
    assert!(kept.final_result().is_none());

    peer.frame(
        "jobTokens",
        json!({"jobID": kept.job_id().to_lowercase(), "content": "kept"}),
    );
    peer.frame(
        "llmJobResult",
        json!({"jobID": kept.job_id(), "timeTaken": 1}),
    );
    assert_eq!(
        tokio::time::timeout(WATCHDOG, kept.wait(None))
            .await
            .unwrap()
            .unwrap()
            .content,
        "kept"
    );
    assert_eq!(kept.next().await.unwrap().unwrap().content, "kept");
    assert!(kept.next().await.is_none());
    // A late result for settled work and a duplicate snapshot cannot emit a
    // second terminal error or completion.
    peer.frame("llmJobResult", json!({"jobID": missing.job_id()}));
    peer.frame("llmJobResult", json!({"jobID": kept.job_id()}));
    peer.frame("authenticated", json!({"activeLLMJobIDs": []}));
    peer.barrier(&client).await;
    let mut terminals = Vec::new();
    while let Ok(event) = events.try_recv() {
        if matches!(event.name.as_str(), "error" | "completed") {
            terminals.push((event.name, event.data["jobID"].as_str().unwrap().to_owned()));
        }
    }
    assert_eq!(
        terminals,
        [
            ("error".into(), missing.job_id().to_owned()),
            ("completed".into(), kept.job_id().to_owned()),
        ]
    );
    while let Ok(event) = transport_events.try_recv() {
        assert!(!matches!(
            event.name.as_str(),
            "connecting" | "disconnected"
        ));
    }
    assert!(
        peer.requests.try_recv().is_err(),
        "reconciliation must not replay requests"
    );
    peer.finish(client).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn initial_handshake_preserves_work_queued_before_it_can_be_written() {
    let (client, mut peer) = fixture().await;
    let request = params();
    let mut queued = (0..8)
        .map(|_| Box::pin(client.chat.stream_completion(&request)))
        .collect::<Vec<_>>();
    for completion in &mut queued {
        assert!(matches!(
            futures_util::poll!(completion.as_mut()),
            Poll::Pending
        ));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), peer.requests.recv())
            .await
            .is_err(),
        "queued work must not be sent before server authentication"
    );
    peer.frame("authenticated", json!({"activeLLMJobIDs": []}));
    let streams = tokio::time::timeout(WATCHDOG, join_all(queued))
        .await
        .expect("queued sends must finish after authentication")
        .into_iter()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    let mut received = std::collections::BTreeSet::new();
    for _ in 0..streams.len() {
        received.insert(peer.request().await["jobID"].as_str().unwrap().to_owned());
    }
    assert_eq!(
        received,
        streams
            .iter()
            .map(|stream| stream.job_id().to_owned())
            .collect::<std::collections::BTreeSet<_>>()
    );
    peer.barrier(&client).await;
    for stream in &streams {
        peer.frame(
            "jobTokens",
            json!({"jobID": stream.job_id(), "content": "queued"}),
        );
        peer.frame("llmJobResult", json!({"jobID": stream.job_id()}));
    }
    for stream in streams {
        assert_eq!(
            tokio::time::timeout(WATCHDOG, stream.wait(None))
                .await
                .unwrap()
                .unwrap()
                .content,
            "queued"
        );
    }
    assert!(peer.requests.try_recv().is_err());
    peer.finish(client).await;
}
