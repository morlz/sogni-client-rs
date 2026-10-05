use axum::{Json, Router, routing::get};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use super::*;
use crate::{
    Event, SogniClient,
    utils::{b64_json_decode, b64_json_encode},
};

const MEDIA: &str = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
const IMAGE: &str = "BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB";

fn frame(name: &str, data: Value) -> Message {
    Message::Text(
        json!({"type":name,"data":b64_json_encode(&data).unwrap()})
            .to_string()
            .into(),
    )
}

async fn next_project_event(receiver: &mut crate::EventReceiver) -> Event {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = receiver.recv().await.unwrap();
            if event.name == "project" {
                return event;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn shared_socket_routes_chat_startup_without_creating_media_jobs() {
    let app = Router::new()
        .route("/v1/account/me", get(|| async {
            Json(json!({"status":"success","data":{"username":"fixture","walletAddress":"fixture"}}))
        }))
        .route("/api/v1/artist/projects/sync", get(|| async {
            Json(json!({"activeProjects":[],"unclaimedCompletedProjects":[]}))
        }))
        .route("/api/v1/artist/projects/active", get(|| async { Json(json!({"projects":[]})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_address = listener.local_addr().unwrap();
    let http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socket_address = listener.local_addr().unwrap();
    let socket = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket
            .send(frame(
                "authenticated",
                json!({"username":"fixture","address":"fixture","subscriptionEntitlement":{}}),
            ))
            .await
            .unwrap();
        while let Some(Ok(message)) = socket.next().await {
            match message {
                Message::Text(text) => {
                    let envelope: Value = serde_json::from_str(&text).unwrap();
                    if envelope["type"] != "llmJobRequest" {
                        continue;
                    }
                    let request = b64_json_decode(envelope["data"].as_str().unwrap()).unwrap();
                    let job = request["jobID"].clone();
                    for (name, data) in [
                        (
                            "jobState",
                            json!({"type":"initiatingModel","jobID":job,"workerName":"chat-worker"}),
                        ),
                        (
                            "jobState",
                            json!({"type":"jobStarted","jobID":job,"workerName":"chat-worker"}),
                        ),
                        (
                            "jobState",
                            json!({"type":"jobStarted","jobID":"OTHER-TAB-CHAT","workerName":"other-worker"}),
                        ),
                        // Missing render ids must stay harmless even for a known project id.
                        (
                            "jobState",
                            json!({"type":"initiatingModel","jobID":MEDIA,"workerName":"chat-worker"}),
                        ),
                        (
                            "jobState",
                            json!({"type":"jobStarted","jobID":MEDIA,"workerName":"chat-worker"}),
                        ),
                        (
                            "jobState",
                            json!({"type":"initiatingModel","jobID":MEDIA,"imgID":IMAGE,"workerName":"media-worker"}),
                        ),
                        (
                            "jobState",
                            json!({"type":"jobStarted","jobID":MEDIA,"imgID":IMAGE,"workerName":"media-worker"}),
                        ),
                        (
                            "jobTokens",
                            json!({"jobID":job,"content":"fixture response"}),
                        ),
                        ("llmJobResult", json!({"jobID":job,"timeTaken":1})),
                    ] {
                        socket.send(frame(name, data)).await.unwrap();
                    }
                }
                Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                Message::Close(_) => break,
                _ => {}
            }
        }
    });
    let client = SogniClient::builder()
        .app_id("local-startup-isolation-fixture")
        .api_key("local-fixture")
        .rest_endpoint(Url::parse(&format!("http://{http_address}/")).unwrap())
        .socket_endpoint(Url::parse(&format!("ws://{socket_address}/")).unwrap())
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(2))
        .build()
        .await
        .unwrap();
    let project = Project::new(
        MEDIA.into(),
        json!({"type":"image","numberOfMedia":1}),
        false,
        Arc::downgrade(&client.projects.inner),
    );
    client
        .projects
        .inner
        .projects
        .write()
        .insert(MEDIA.into(), project.clone());
    let mut media_events = client.projects.subscribe();
    let mut chat_events = client.chat.subscribe();
    let stream = client
        .chat
        .stream_completion(
            &json!({"model":"fixture-llm","messages":[{"role":"user","content":"fixture"}]}),
        )
        .await
        .unwrap();
    let completion = stream.wait(Some(Duration::from_secs(2))).await.unwrap();
    assert_eq!(completion.content, "fixture response");
    assert_eq!(completion.worker_name.as_deref(), Some("chat-worker"));
    let first = next_project_event(&mut media_events).await;
    let second = next_project_event(&mut media_events).await;
    assert_eq!(first.data["type"], "initiatingModel");
    assert_eq!(second.data["type"], "jobStarted");
    for event in [first, second] {
        assert_eq!(event.data["jobID"], MEDIA);
        assert_eq!(event.data["imgID"], IMAGE);
    }
    assert_eq!(project.jobs().len(), 1);
    assert_eq!(project.jobs()[0].id(), IMAGE);
    assert_eq!(
        project.jobs()[0].snapshot().worker_name.as_deref(),
        Some("media-worker")
    );
    assert_eq!(project.status(), ProjectStatus::Processing);
    let mut chat_states = Vec::new();
    while let Ok(event) = chat_events.try_recv() {
        if event.name == "jobState" {
            chat_states.push(event.data);
        }
    }
    assert_eq!(chat_states.len(), 2);
    assert_eq!(chat_states[0]["type"], "initiatingModel");
    assert_eq!(chat_states[1]["type"], "jobStarted");
    assert!(
        chat_states
            .iter()
            .all(|state| state["jobID"] == stream.job_id())
    );
    client.close().await.unwrap();
    http.abort();
    socket.abort();
}
