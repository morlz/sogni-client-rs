use super::policy::ABNORMAL_CLOSURE;
use super::*;

#[test]
fn eof_and_connection_reset_are_reconnectable_transport_losses() {
    for reason in ["WebSocket stream ended", "connection reset by peer"] {
        let SocketOutcome::Disconnected {
            code,
            reason: actual_reason,
        } = transport_loss(reason)
        else {
            panic!("transport loss must produce a disconnected outcome");
        };

        assert_eq!(code, ABNORMAL_CLOSURE);
        assert_eq!(actual_reason, reason);
        assert_eq!(
            disconnect_disposition(code),
            DisconnectDisposition::Reconnect
        );
    }
}

#[test]
fn intentional_and_terminal_closes_keep_existing_semantics() {
    assert_eq!(disconnect_disposition(1000), DisconnectDisposition::Suspend);
    assert_eq!(
        disconnect_disposition(SWITCH_CONNECTION),
        DisconnectDisposition::Suspend
    );
    assert_eq!(
        disconnect_disposition(0),
        DisconnectDisposition::ClearAuthAndSuspend
    );
    assert_eq!(
        disconnect_disposition(4001),
        DisconnectDisposition::ClearAuthAndSuspend
    );
}

#[tokio::test]
async fn closing_one_account_keeps_new_account_commands() {
    let (commands, mut receiver) = mpsc::channel(4);
    let mut pending = VecDeque::new();
    let mut results = Vec::new();
    for (session, queued) in [(7, false), (8, false), (7, true), (8, true)] {
        let (response, waiter) = tokio::sync::oneshot::channel();
        let command = SocketCommand::Send {
            message: Message::Text("fixture".into()),
            response,
            session,
        };
        results.push((session, waiter));
        if queued {
            commands.send(command).await.unwrap();
        } else {
            pending.push_back(command);
        }
    }
    fail_session(&mut pending, &mut receiver, 7);
    assert_eq!(pending.len(), 2);
    for (session, mut result) in results {
        if session == 7 {
            assert!(result.await.is_err());
        } else {
            assert_eq!(
                result.try_recv().unwrap_err(),
                tokio::sync::oneshot::error::TryRecvError::Empty
            );
        }
    }
}
