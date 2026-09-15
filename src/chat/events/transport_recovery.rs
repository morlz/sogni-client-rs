use super::*;
use crate::Error;
use std::{collections::HashSet, time::Duration};

const TRANSPORT_LOST_MESSAGE: &str =
    "The connection to Sogni dropped and this request did not survive it. Send it again.";

#[derive(Default)]
pub(in crate::chat) struct TransportRecovery {
    pub(in crate::chat) unsent: HashSet<String>,
    awaiting: HashSet<String>,
    terminal_epoch: u64,
    timer_epoch: u64,
    timer_active: bool,
}

impl TransportRecovery {
    pub(in crate::chat) fn submitting(&mut self, id: String) -> u64 {
        self.unsent.insert(id);
        self.terminal_epoch
    }
}

pub(in crate::chat) fn submitted(inner: &ChatInner, id: &str, terminal_epoch: u64) {
    let closed_during_send = {
        let mut recovery = inner.recovery.lock();
        recovery.unsent.remove(id);
        recovery.terminal_epoch != terminal_epoch
    };
    // The socket can acknowledge a write and close before the submitter runs
    // again. Such work was skipped as unsent by closed(), but cannot survive a
    // terminal close. Ordinary reconnects leave this generation unchanged.
    if closed_during_send {
        fail(inner, id);
    }
}

pub(in crate::chat) fn transport_error(job_id: &str, cause: Option<&Error>) -> ChatError {
    let message = cause.map_or_else(
        || TRANSPORT_LOST_MESSAGE.to_owned(),
        |cause| format!("{TRANSPORT_LOST_MESSAGE} ({cause})"),
    );
    ChatError::from_payload(
        json!({"error":"transport_lost", "error_message":message}),
        None,
        Some(job_id.to_owned()),
    )
}

pub(super) fn lost(inner: &Arc<ChatInner>) {
    let active: Vec<_> = inner.active.read().keys().cloned().collect();
    let epoch = {
        let mut recovery = inner.recovery.lock();
        for id in active {
            if !recovery.unsent.contains(&id) {
                recovery.awaiting.insert(id);
            }
        }
        if recovery.awaiting.is_empty() || recovery.timer_active {
            return;
        }
        recovery.timer_active = true;
        recovery.timer_epoch = recovery.timer_epoch.wrapping_add(1);
        recovery.timer_epoch
    };
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        // A peer reconnect may restore this stream. Bound silence even when an
        // older peer cannot report surviving job ids in its authentication frame.
        tokio::time::sleep(Duration::from_secs(35)).await;
        if let Some(inner) = weak.upgrade() {
            expire(&inner, epoch);
        }
    });
}

pub(super) fn closed(inner: &Arc<ChatInner>, data: &Value) {
    let terminal = data.get("code").and_then(Value::as_u64) != Some(4015);
    if terminal {
        let mut recovery = inner.recovery.lock();
        recovery.terminal_epoch = recovery.terminal_epoch.wrapping_add(1);
    }
    lost(inner);
    if terminal {
        let epoch = inner.recovery.lock().timer_epoch;
        expire(inner, epoch);
    }
}

fn expire(inner: &ChatInner, epoch: u64) {
    let ids = {
        let mut recovery = inner.recovery.lock();
        if recovery.timer_epoch != epoch || !recovery.timer_active {
            return;
        }
        recovery.timer_active = false;
        std::mem::take(&mut recovery.awaiting)
    };
    for id in ids {
        fail(inner, &id);
    }
}

pub(super) fn authenticated(inner: &ChatInner, data: &Value) {
    let Some(ids) = data.get("activeLLMJobIDs").and_then(Value::as_array) else {
        return;
    };
    let live: HashSet<_> = ids
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_uppercase)
        .collect();
    let gone = {
        let mut recovery = inner.recovery.lock();
        recovery.timer_active = false;
        recovery.timer_epoch = recovery.timer_epoch.wrapping_add(1);
        std::mem::take(&mut recovery.awaiting)
            .into_iter()
            .filter(|id| !live.contains(&id.to_uppercase()))
            .collect::<Vec<_>>()
    };
    for id in gone {
        fail(inner, &id);
    }
}

pub(super) fn alive(inner: &ChatInner, id: &str) {
    let mut recovery = inner.recovery.lock();
    if recovery.awaiting.remove(id) && recovery.awaiting.is_empty() {
        recovery.timer_active = false;
        recovery.timer_epoch = recovery.timer_epoch.wrapping_add(1);
    }
}

fn fail(inner: &ChatInner, id: &str) {
    handle_error(
        inner,
        &json!({"jobID":id,"error":"transport_lost",
        "error_message":TRANSPORT_LOST_MESSAGE}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SogniClient;
    use crate::chat::events::tests::active_chat;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn terminal_close_before_send_completion_settles_unbounded_stream_wait() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let (chat, mut stream) = active_chat("JOB");
        let terminal_epoch = inner.recovery.lock().submitting("JOB".into());
        inner.active.write().insert("JOB".into(), chat);

        // Model a successful socket write whose acknowledgement is ready, but
        // whose submitter has not resumed when the terminal close is handled.
        closed(inner, &json!({"code":1000}));
        assert!(!stream.state.read().complete);
        submitted(inner, "JOB", terminal_epoch);

        let error = tokio::time::timeout(Duration::from_millis(50), stream.wait(None))
            .await
            .expect("acknowledged work must not wait forever after a terminal close")
            .unwrap_err();
        assert!(crate::is_retryable_chat_error(&error));
        assert!(crate::is_retryable_chat_error(
            &stream.next().await.unwrap().unwrap_err()
        ));
        assert!(stream.next().await.is_none());
        assert!(inner.active.read().is_empty());
        assert!(inner.recovery.lock().unsent.is_empty());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn interrupted_close_preserves_queued_send_and_confirmed_survivor() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let (sent, sent_stream) = active_chat("SENT");
        let (queued, queued_stream) = active_chat("QUEUED");
        let terminal_epoch = inner.recovery.lock().submitting("QUEUED".into());
        inner
            .active
            .write()
            .extend([("SENT".into(), sent), ("QUEUED".into(), queued)]);

        closed(inner, &json!({"code":4015}));
        let grace_epoch = {
            let recovery = inner.recovery.lock();
            assert!(recovery.timer_active);
            assert!(recovery.awaiting.contains("SENT"));
            assert!(!recovery.awaiting.contains("QUEUED"));
            recovery.timer_epoch
        };
        assert!(!sent_stream.state.read().complete);
        assert!(!queued_stream.state.read().complete);

        authenticated(inner, &json!({"activeLLMJobIDs":["sent"]}));
        submitted(inner, "QUEUED", terminal_epoch);
        expire(inner, grace_epoch);
        assert!(!sent_stream.state.read().complete);
        assert!(!queued_stream.state.read().complete);
        assert!(inner.recovery.lock().unsent.is_empty());
        assert_eq!(inner.active.read().len(), 2);

        for id in ["SENT", "QUEUED"] {
            handle_result(inner, &json!({"jobID":id}));
        }
        assert!(sent_stream.wait(None).await.is_ok());
        assert!(queued_stream.wait(None).await.is_ok());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn reconnect_keeps_confirmed_streams_and_rejects_missing_without_replay() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let (present, present_stream) = active_chat("PRESENT");
        let (gone, gone_stream) = active_chat("GONE");
        let (unsent, unsent_stream) = active_chat("UNSENT");
        inner.active.write().extend([
            ("PRESENT".into(), present),
            ("GONE".into(), gone),
            ("UNSENT".into(), unsent),
        ]);
        inner.recovery.lock().unsent.insert("UNSENT".into());
        lost(inner);
        authenticated(inner, &json!({"activeLLMJobIDs":["present"]}));
        let error = gone_stream
            .wait(Some(Duration::from_millis(50)))
            .await
            .unwrap_err();
        assert!(crate::is_retryable_chat_error(&error));
        assert!(!present_stream.state.read().complete);
        assert!(!unsent_stream.state.read().complete);
        assert_eq!(inner.active.read().len(), 2);
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn frames_cancel_stale_timers_and_terminal_closes_settle_streams() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let (chat, stream) = active_chat("JOB");
        inner.active.write().insert("JOB".into(), chat);
        lost(inner);
        let stale = inner.recovery.lock().timer_epoch;
        handle_tokens(inner, &json!({"jobID":"JOB","content":"resumed"}));
        expire(inner, stale);
        assert_eq!(stream.state.read().content, "resumed");
        assert!(!stream.state.read().complete);
        closed(inner, &json!({"code":1000}));
        assert!(crate::is_retryable_chat_error(
            &stream.wait(None).await.unwrap_err()
        ));
        client.close().await.unwrap();
    }
}
