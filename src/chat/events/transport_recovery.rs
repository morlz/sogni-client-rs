use super::*;
use crate::Error;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

const TRANSPORT_LOST_MESSAGE: &str =
    "The connection to Sogni dropped and this request did not survive it. Send it again.";

#[derive(Default)]
pub(in crate::chat) struct TransportRecovery {
    unsent: HashMap<String, PendingSend>,
    sent: HashMap<String, u64>,
    awaiting: HashMap<String, u64>,
    timer_epoch: u64,
}

struct PendingSend {
    session: u64,
    closed: bool,
    handshake: Option<Handshake>,
}

#[derive(Clone, Copy)]
struct Handshake {
    sequence: u64,
    kept: bool,
}

impl TransportRecovery {
    pub(in crate::chat) fn submitting(&mut self, id: String, session: u64) {
        self.unsent.insert(
            id,
            PendingSend {
                session,
                closed: false,
                handshake: None,
            },
        );
    }

    pub(in crate::chat) fn forget(&mut self, id: &str) {
        self.unsent.remove(id);
        self.sent.remove(id);
        self.awaiting.remove(id);
    }
}

pub(in crate::chat) fn submitted(inner: &ChatInner, id: &str, sent_after_event: u64) {
    let ended_during_send = {
        let mut recovery = inner.recovery.lock();
        let Some(pending) = recovery.unsent.remove(id) else {
            // A terminal frame may finish the stream before the submitter wakes.
            return;
        };
        let ended = pending.closed
            || pending
                .handshake
                .is_some_and(|handshake| !handshake.kept && sent_after_event < handshake.sequence);
        if !ended {
            recovery.sent.insert(id.to_owned(), sent_after_event);
        }
        ended
    };
    // The socket can acknowledge a write and close before the submitter runs
    // again. Such work was skipped as unsent by closed(), but cannot survive a
    // terminal close of its own account. Another session's delayed close must
    // not invalidate a successful submission on the new account's socket.
    // A handshake can likewise arrive before the write acknowledgement is
    // consumed. Only writes before that frame are eligible for reconciliation.
    if ended_during_send {
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

pub(super) fn lost(inner: &Arc<ChatInner>, session: Option<u64>) {
    let active: Vec<_> = inner
        .active
        .read()
        .iter()
        .filter(|(_, chat)| session.is_none_or(|session| chat.session == session))
        .map(|(id, _)| id.clone())
        .collect();
    let epoch = {
        let mut recovery = inner.recovery.lock();
        let epoch = recovery.timer_epoch.wrapping_add(1);
        let mut added = false;
        for id in active {
            if !recovery.unsent.contains_key(&id) && !recovery.awaiting.contains_key(&id) {
                recovery.awaiting.insert(id, epoch);
                added = true;
            }
        }
        if !added {
            return;
        }
        recovery.timer_epoch = epoch;
        epoch
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

pub(super) fn closed(inner: &Arc<ChatInner>, data: &Value, session: Option<u64>) {
    let terminal = data.get("code").and_then(Value::as_u64) != Some(4015);
    if !terminal {
        lost(inner, session);
        return;
    }
    let active: Vec<_> = inner
        .active
        .read()
        .iter()
        .filter(|(_, chat)| session.is_none_or(|session| chat.session == session))
        .map(|(id, _)| id.clone())
        .collect();
    let ids = {
        let mut recovery = inner.recovery.lock();
        for pending in recovery.unsent.values_mut() {
            if session.is_none_or(|session| pending.session == session) {
                pending.closed = true;
            }
        }
        active
            .into_iter()
            .filter(|id| !recovery.unsent.contains_key(id))
            .collect::<Vec<_>>()
    };
    for id in ids {
        fail(inner, &id);
    }
}

fn expire(inner: &ChatInner, epoch: u64) {
    let ids = {
        let mut recovery = inner.recovery.lock();
        let ids = recovery
            .awaiting
            .iter()
            .filter(|(_, pending_epoch)| **pending_epoch == epoch)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in &ids {
            recovery.awaiting.remove(id);
        }
        ids
    };
    for id in ids {
        fail(inner, &id);
    }
}

pub(super) fn authenticated(inner: &ChatInner, data: &Value, session: Option<u64>, sequence: u64) {
    let Some(ids) = data.get("activeLLMJobIDs").and_then(Value::as_array) else {
        return;
    };
    let live: HashSet<_> = ids
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_uppercase)
        .collect();
    let active: Vec<_> = inner
        .active
        .read()
        .iter()
        .filter(|(_, chat)| session.is_none_or(|session| chat.session == session))
        .map(|(id, _)| id.clone())
        .collect();
    let gone = {
        let mut recovery = inner.recovery.lock();
        let mut gone = Vec::new();
        for id in active {
            recovery.awaiting.remove(&id);
            let kept = live.contains(&id.to_uppercase());
            if let Some(pending) = recovery.unsent.get_mut(&id) {
                if pending
                    .handshake
                    .is_none_or(|previous| previous.sequence < sequence)
                {
                    pending.handshake = Some(Handshake { sequence, kept });
                }
            } else if !kept
                && recovery
                    .sent
                    .get(&id)
                    .is_none_or(|sent_after_event| *sent_after_event < sequence)
            {
                gone.push(id);
            }
        }
        gone
    };
    for id in gone {
        fail(inner, &id);
    }
}

pub(super) fn alive(inner: &ChatInner, id: &str) {
    inner.recovery.lock().awaiting.remove(id);
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
    async fn missing_handshake_settles_an_earlier_write_with_a_delayed_acknowledgement() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let (chat, stream) = active_chat("EARLIER");
        inner.recovery.lock().submitting("EARLIER".into(), 0);
        inner.active.write().insert("EARLIER".into(), chat);

        // The writer completed before frame 2, but its caller has not resumed.
        authenticated(inner, &json!({"activeLLMJobIDs": []}), None, 2);
        submitted(inner, "EARLIER", 1);
        assert!(crate::is_retryable_chat_error(
            &stream.wait(None).await.unwrap_err()
        ));
        assert!(inner.active.read().is_empty());
        assert!(inner.recovery.lock().sent.is_empty());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn delayed_handshake_does_not_reject_a_write_that_followed_its_frame() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let (chat, stream) = active_chat("LATER");
        inner.recovery.lock().submitting("LATER".into(), 0);
        inner.active.write().insert("LATER".into(), chat);

        // Frame 2 unblocked the writer. Its acknowledgement reached the caller
        // before the separate chat listener consumed that same frame.
        submitted(inner, "LATER", 2);
        authenticated(inner, &json!({"activeLLMJobIDs": []}), None, 2);
        assert!(!stream.state.read().complete);
        handle_result(inner, &json!({"jobID": "LATER"}));
        assert!(stream.wait(None).await.is_ok());
        assert!(inner.recovery.lock().sent.is_empty());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn terminal_result_before_a_late_receipt_does_not_recreate_recovery_state() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let (chat, stream) = active_chat("DONE");
        inner.recovery.lock().submitting("DONE".into(), 0);
        inner.active.write().insert("DONE".into(), chat);
        let mut events = inner.events.subscribe();

        authenticated(inner, &json!({"activeLLMJobIDs": []}), None, 2);
        handle_result(inner, &json!({"jobID": "DONE"}));
        submitted(inner, "DONE", 1);
        assert!(stream.wait(None).await.is_ok());
        assert_eq!(events.try_recv().unwrap().name, "completed");
        assert!(events.try_recv().is_err());
        {
            let recovery = inner.recovery.lock();
            assert!(
                recovery.unsent.is_empty()
                    && recovery.sent.is_empty()
                    && recovery.awaiting.is_empty()
            );
        }
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_later_kept_handshake_clears_a_pending_missing_decision() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let (chat, stream) = active_chat("KEPT");
        inner.recovery.lock().submitting("KEPT".into(), 0);
        inner.active.write().insert("KEPT".into(), chat);

        authenticated(inner, &json!({"activeLLMJobIDs": []}), None, 2);
        authenticated(inner, &json!({"activeLLMJobIDs": ["kept"]}), None, 3);
        authenticated(inner, &json!({"activeLLMJobIDs": []}), None, 2);
        submitted(inner, "KEPT", 1);
        assert!(!stream.state.read().complete);
        handle_result(inner, &json!({"jobID": "KEPT"}));
        assert!(stream.wait(None).await.is_ok());
        assert!(inner.recovery.lock().sent.is_empty());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn terminal_close_before_send_completion_settles_unbounded_stream_wait() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let (chat, mut stream) = active_chat("JOB");
        inner.recovery.lock().submitting("JOB".into(), 0);
        inner.active.write().insert("JOB".into(), chat);

        // Model a successful socket write whose acknowledgement is ready, but
        // whose submitter has not resumed when the terminal close is handled.
        closed(inner, &json!({"code":1000}), None);
        assert!(!stream.state.read().complete);
        submitted(inner, "JOB", 0);

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
        inner.recovery.lock().submitting("QUEUED".into(), 0);
        inner
            .active
            .write()
            .extend([("SENT".into(), sent), ("QUEUED".into(), queued)]);

        closed(inner, &json!({"code":4015}), None);
        let grace_epoch = {
            let recovery = inner.recovery.lock();
            assert!(recovery.awaiting.contains_key("SENT"));
            assert!(!recovery.awaiting.contains_key("QUEUED"));
            recovery.timer_epoch
        };
        assert!(!sent_stream.state.read().complete);
        assert!(!queued_stream.state.read().complete);

        authenticated(inner, &json!({"activeLLMJobIDs":["sent"]}), None, 1);
        submitted(inner, "QUEUED", 1);
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
        inner.recovery.lock().submitting("UNSENT".into(), 0);
        lost(inner, None);
        authenticated(inner, &json!({"activeLLMJobIDs":["present"]}), None, 1);
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
        lost(inner, None);
        let stale = inner.recovery.lock().timer_epoch;
        handle_tokens(inner, &json!({"jobID":"JOB","content":"resumed"}));
        expire(inner, stale);
        assert_eq!(stream.state.read().content, "resumed");
        assert!(!stream.state.read().complete);
        closed(inner, &json!({"code":1000}), None);
        assert!(crate::is_retryable_chat_error(
            &stream.wait(None).await.unwrap_err()
        ));
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn delayed_old_session_events_preserve_new_account_chats_and_recovery() {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let inner = &client.chat.inner;
        let wire_events = EventBus::default();
        let mut delayed_listener = wire_events.subscribe_scoped();
        let mut public_listener = wire_events.subscribe();
        let mut completions = inner.events.subscribe();
        let (mut old_sent, old_sent_stream) = active_chat("OLD_SENT");
        let (mut old_queued, old_queued_stream) = active_chat("OLD_QUEUED");
        old_sent.session = 7;
        old_queued.session = 7;
        inner.recovery.lock().submitting("OLD_QUEUED".into(), 7);
        inner.active.write().extend([
            ("OLD_SENT".into(), old_sent),
            ("OLD_QUEUED".into(), old_queued),
        ]);
        lost(inner, Some(7));
        let old_timer = inner.recovery.lock().timer_epoch;

        // These events originate on A's socket, but the asynchronous listener
        // does not consume them until B has already registered and sent work.
        wire_events.emit_scoped("connecting", json!({"network":"fast"}), 7);
        wire_events.emit_scoped("authenticated", json!({"activeLLMJobIDs":[]}), 7);
        let close_data = json!({"code":0,"reason":"Authentication changed"});
        wire_events.emit_scoped("disconnected", close_data.clone(), 7);

        let (mut new_sent, mut new_sent_stream) = active_chat("NEW_SENT");
        let (mut new_queued, mut new_queued_stream) = active_chat("NEW_QUEUED");
        new_sent.session = 8;
        new_queued.session = 8;
        inner.recovery.lock().submitting("NEW_QUEUED".into(), 8);
        inner.active.write().extend([
            ("NEW_SENT".into(), new_sent),
            ("NEW_QUEUED".into(), new_queued),
        ]);
        lost(inner, Some(8));
        let new_timer = inner.recovery.lock().awaiting["NEW_SENT"];
        assert_ne!(old_timer, new_timer);
        for _ in 0..3 {
            super::super::handle_chat_event(inner, delayed_listener.try_recv().unwrap());
        }
        // Successful send acknowledgements arrive after the delayed close.
        submitted(inner, "OLD_QUEUED", wire_events.sequence());
        submitted(inner, "NEW_QUEUED", wire_events.sequence());
        assert!(crate::is_retryable_chat_error(
            &old_sent_stream.wait(None).await.unwrap_err()
        ));
        assert!(crate::is_retryable_chat_error(
            &old_queued_stream.wait(None).await.unwrap_err()
        ));
        assert!(!new_sent_stream.state.read().complete);
        assert!(!new_queued_stream.state.read().complete);
        assert_eq!(inner.recovery.lock().awaiting["NEW_SENT"], new_timer);
        expire(inner, old_timer);
        assert!(!new_sent_stream.state.read().complete);
        assert!(inner.recovery.lock().awaiting.contains_key("NEW_SENT"));

        authenticated(
            inner,
            &json!({"activeLLMJobIDs":["NEW_SENT", "NEW_QUEUED"]}),
            Some(8),
            wire_events.sequence() + 1,
        );
        expire(inner, new_timer);
        for id in ["NEW_SENT", "NEW_QUEUED"] {
            handle_tokens(inner, &json!({"jobID":id,"content":"done"}));
            handle_result(inner, &json!({"jobID":id}));
        }
        for stream in [&mut new_sent_stream, &mut new_queued_stream] {
            assert_eq!(stream.wait(None).await.unwrap().content, "done");
            assert_eq!(stream.next().await.unwrap().unwrap().content, "done");
            assert!(stream.next().await.is_none());
        }
        let mut completed = Vec::new();
        while let Ok(event) = completions.try_recv() {
            if event.name == "completed" {
                completed.push(event.data["jobID"].as_str().unwrap().to_owned());
            }
        }
        assert_eq!(completed, ["NEW_SENT", "NEW_QUEUED"]);
        // Public events keep their original payload; provenance stays internal.
        public_listener.try_recv().unwrap();
        public_listener.try_recv().unwrap();
        assert_eq!(public_listener.try_recv().unwrap().data, close_data);
        assert!(inner.active.read().is_empty());
        assert!(inner.recovery.lock().unsent.is_empty());
        assert!(inner.recovery.lock().awaiting.is_empty());
        client.close().await.unwrap();
    }
}
