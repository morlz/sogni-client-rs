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
