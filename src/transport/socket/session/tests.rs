use super::policy::ABNORMAL_CLOSURE;
use super::*;

#[tokio::test]
async fn expired_command_is_not_written_even_before_its_caller_timeout_runs() {
    use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};
    let (left, right) = tokio::io::duplex(1024);
    let mut writer = WebSocketStream::from_raw_socket(left, Role::Client, None).await;
    let mut reader = WebSocketStream::from_raw_socket(right, Role::Server, None).await;
    let (response, waiter) = tokio::sync::oneshot::channel();
    let result = send_command(
        &mut writer,
        SocketCommand::Send {
            message: Message::Text("expired request".into()),
            response,
            session: 7,
            deadline: tokio::time::Instant::now() - Duration::from_secs(1),
        },
        7,
        1,
    )
    .await;
    assert!(result.is_ok());
    assert!(matches!(waiter.await.unwrap(), Err(Error::Timeout(_))));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), reader.next())
            .await
            .is_err()
    );
}

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
            deadline: tokio::time::Instant::now() + Duration::from_secs(30),
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
