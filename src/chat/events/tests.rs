use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    task::Poll,
    time::Duration,
};

use futures_util::StreamExt;
use parking_lot::RwLock;
use serde_json::json;
use tokio::sync::{Notify, mpsc};

use super::*;
use crate::{
    Error,
    chat::types::{ChatStream, ChatStreamState},
    event::EventBus,
};

pub(super) fn active_chat(job_id: &str) -> (ActiveChat, ChatStream) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let state = Arc::new(RwLock::new(ChatStreamState::default()));
    let changed = Arc::new(Notify::new());
    (
        ActiveChat {
            session: 0,
            sender,
            state: state.clone(),
            changed: changed.clone(),
        },
        ChatStream {
            job_id: job_id.to_owned(),
            receiver,
            state,
            changed,
        },
    )
}

fn assert_lag_error(error: Error, job_id: &str) {
    let Error::Chat(error) = error else {
        panic!("expected chat lag error, got {error}");
    };
    assert_eq!(error.code.as_deref(), Some(CHAT_EVENT_LAG_CODE));
    assert_eq!(error.error_type.as_deref(), Some(CHAT_EVENT_LAG_TYPE));
    assert_eq!(error.job_id.as_deref(), Some(job_id));
    assert_eq!(error.message, CHAT_EVENT_LAG_MESSAGE);
}

#[test]
fn merges_streamed_tool_arguments() {
    let mut calls = BTreeMap::new();
    merge_tool_call_delta(
        &mut calls,
        &json!({"index": 0, "id": "c", "function": {"name": "x", "arguments": "{\"a\":"}}),
    );
    merge_tool_call_delta(
        &mut calls,
        &json!({"index": 0, "function": {"arguments": "1}"}}),
    );
    assert_eq!(calls[&0]["function"]["arguments"], "{\"a\":1}");
}

#[tokio::test]
async fn lag_terminates_non_streaming_completion_wait() {
    let active = RwLock::new(HashMap::new());
    let events = EventBus::default();
    let (tracked, stream) = active_chat("non-streaming-job");
    active.write().insert(stream.job_id.clone(), tracked);
    let mut wait = Box::pin(stream.wait(Some(Duration::from_secs(300))));
    assert!(matches!(futures_util::poll!(wait.as_mut()), Poll::Pending));

    let affected = take_active_snapshot(&active);
    fail_lagged_snapshot(&events, affected);

    let error = wait
        .await
        .expect_err("lag must fail the completion without waiting for its timeout");
    assert_lag_error(error, "non-streaming-job");
    assert!(active.read().is_empty());
}

#[tokio::test]
async fn lag_terminates_unbounded_stream_wait_and_preserves_new_jobs() {
    let active = RwLock::new(HashMap::new());
    let events = EventBus::default();
    let (lagged, mut lagged_stream) = active_chat("lagged-job");
    active.write().insert(lagged_stream.job_id.clone(), lagged);
    let mut wait = Box::pin(lagged_stream.wait(None));
    assert!(matches!(futures_util::poll!(wait.as_mut()), Poll::Pending));

    let affected = take_active_snapshot(&active);
    let (new, new_stream) = active_chat("new-job");
    active.write().insert(new_stream.job_id.clone(), new);
    fail_lagged_snapshot(&events, affected);

    let error = wait
        .await
        .expect_err("an unbounded wait must terminate after lag");
    assert_lag_error(error, "lagged-job");
    let stream_error = lagged_stream
        .next()
        .await
        .expect("lagged stream must receive a terminal error")
        .expect_err("lagged stream item must be an error");
    assert_lag_error(stream_error, "lagged-job");
    assert!(lagged_stream.next().await.is_none());
    assert!(active.read().contains_key("new-job"));
    assert!(!new_stream.state.read().complete);
    assert!(new_stream.state.read().error.is_none());
}

fn session_chat(endpoint: url::Url) -> (crate::ChatApi, crate::auth::AuthManager) {
    let http = crate::transport::HttpClients::build(Duration::from_secs(30)).unwrap();
    let auth = crate::auth::AuthManager::new(
        crate::AuthKind::ApiKey,
        endpoint.clone(),
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("A").unwrap();
    let client = Arc::new(
        crate::transport::ApiClient::new(
            crate::ClientConfig {
                rest_endpoint: endpoint,
                auth_kind: crate::AuthKind::ApiKey,
                disable_socket: true,
                ..Default::default()
            },
            auth.clone(),
            http,
        )
        .unwrap(),
    );
    let projects = crate::ProjectsApi::new(client.clone());
    (crate::ChatApi::new(client, projects), auth)
}

#[tokio::test]
async fn account_session_end_terminates_active_and_queued_chats_without_retry_advice() {
    let (api, auth) = session_chat("http://127.0.0.1:9/".parse().unwrap());
    let inner = &api.inner;
    let (mut active, mut stream) = active_chat("SENT");
    let (mut queued, queued_stream) = active_chat("QUEUED");
    active.session = auth.version().session;
    queued.session = active.session;
    inner
        .recovery
        .lock()
        .submitting("QUEUED".into(), queued.session);
    inner
        .active
        .write()
        .extend([("SENT".into(), active), ("QUEUED".into(), queued)]);
    auth.authenticate_api_key("A").unwrap();
    tokio::task::yield_now().await;
    assert!(!stream.state.read().complete && !queued_stream.state.read().complete);
    auth.authenticate_api_key("B").unwrap();
    for pending in [&stream, &queued_stream] {
        let error = tokio::time::timeout(Duration::from_secs(5), pending.wait(None))
            .await
            .unwrap()
            .unwrap_err();
        assert!(
            matches!(&error,Error::Chat(error) if error.error_type.as_deref() == Some("session_ended"))
        );
        assert!(!crate::is_retryable_chat_error(&error));
    }
    assert!(stream.next().await.unwrap().is_err());
    assert!(stream.next().await.is_none());
    assert!(inner.active.read().is_empty());
    inner.client.close().await.unwrap();
}

#[tokio::test]
async fn client_closure_ends_active_and_queued_chats_without_account_change_or_retry() {
    for abort in [false, true] {
        let (api, auth) = session_chat("http://127.0.0.1:9/".parse().unwrap());
        let owner = api.inner.client.rest.request_session();
        let (mut active, mut stream) = active_chat("SENT");
        let (mut queued, queued_stream) = active_chat("QUEUED");
        active.session = auth.version().session;
        queued.session = active.session;
        api.inner
            .recovery
            .lock()
            .submitting("QUEUED".into(), queued.session);
        api.inner
            .active
            .write()
            .extend([("SENT".into(), active), ("QUEUED".into(), queued)]);
        if abort {
            api.inner.client.abort();
        } else {
            api.inner.client.close().await.unwrap();
        }
        for pending in [&stream, &queued_stream] {
            let error = tokio::time::timeout(Duration::from_secs(5), pending.wait(None))
                .await
                .unwrap()
                .unwrap_err();
            let Error::Chat(chat) = &error else {
                panic!("a retained chat needs a terminal closed-client result");
            };
            assert_eq!(chat.error_type.as_deref(), Some("client_closed"));
            assert!(chat.message.contains("was closed") && !chat.message.contains("account"));
            assert!(!crate::is_retryable_chat_error(&error));
        }
        assert!(stream.next().await.unwrap().is_err());
        assert!(stream.next().await.is_none());
        assert!(api.inner.active.read().is_empty());
        assert!(matches!(
            session_error(&owner, Some("QUEUED")),
            Error::Closed
        ));
        assert!(matches!(
            api.create_completion(&json!({
                "model":"fixture", "messages":[{"role":"user","content":"hello"}]
            }))
            .await,
            Err(Error::Closed)
        ));
    }
}

#[tokio::test]
async fn buffered_old_chat_payloads_cannot_complete_a_current_account_job() {
    let (api, auth) = session_chat("http://127.0.0.1:9/".parse().unwrap());
    let previous = auth.version().session;
    auth.authenticate_api_key("B").unwrap();
    let (mut active, stream) = active_chat("REUSED");
    active.session = auth.version().session;
    api.inner.active.write().insert("REUSED".into(), active);
    for (name, data) in [
        ("jobTokens", json!({"jobID":"REUSED","content":"old"})),
        ("llmJobResult", json!({"jobID":"REUSED","content":"old"})),
        ("llmJobError", json!({"jobID":"REUSED","error":"old"})),
    ] {
        handle_chat_event(
            &api.inner,
            ScopedEvent {
                event: crate::Event {
                    name: name.into(),
                    data,
                },
                session: Some(previous),
            },
        );
    }
    assert!(stream.state.read().content.is_empty());
    assert!(!stream.state.read().complete);
    handle_chat_event(
        &api.inner,
        ScopedEvent {
            event: crate::Event {
                name: "jobTokens".into(),
                data: json!({"jobID":"REUSED","content":"current"}),
            },
            session: Some(auth.version().session),
        },
    );
    handle_chat_event(
        &api.inner,
        ScopedEvent {
            event: crate::Event {
                name: "llmJobResult".into(),
                data: json!({"jobID":"REUSED","content":"current"}),
            },
            session: Some(auth.version().session),
        },
    );
    assert_eq!(stream.wait(None).await.unwrap().content, "current");
    api.inner.client.close().await.unwrap();
}

#[tokio::test]
async fn session_change_during_vision_preparation_never_dispatches_chat() {
    use axum::{
        Router,
        routing::{get, post},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    for hosted in [false, true] {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let observing = entered.clone();
        let held = release.clone();
        let submissions = Arc::new(AtomicUsize::new(0));
        let counted = submissions.clone();
        let router = Router::new()
            .route(
                "/image",
                get(move || {
                    let observing = observing.clone();
                    let held = held.clone();
                    async move {
                        observing.notify_one();
                        held.notified().await;
                        ([("content-type", "image/png")], "fixture image")
                    }
                }),
            )
            .route(
                "/v1/chat/completions",
                post(move || {
                    let counted = counted.clone();
                    async move {
                        counted.fetch_add(1, Ordering::SeqCst);
                        axum::Json(json!({}))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint: url::Url = format!("http://{}/", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let image = endpoint.join("image").unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let (api, auth) = session_chat(endpoint);
        let preparing = api.clone();
        let pending = tokio::spawn(async move {
            let params = json!({"model":"fixture","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":image}}]}]});
            if hosted {
                preparing
                    .create_hosted_completion(&params)
                    .await
                    .map(|_| ())
            } else {
                preparing.stream_completion(&params).await.map(|_| ())
            }
        });
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        auth.authenticate_api_key("B").unwrap();
        let error = tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        if !hosted {
            assert!(
                matches!(&error,Error::Chat(error) if error.error_type.as_deref()==Some("session_ended"))
            );
            assert!(!crate::is_retryable_chat_error(&error));
        }
        assert!(api.inner.active.read().is_empty());
        assert_eq!(submissions.load(Ordering::SeqCst), 0);
        release.notify_one();
        api.inner.client.close().await.unwrap();
        server.abort();
    }
}
