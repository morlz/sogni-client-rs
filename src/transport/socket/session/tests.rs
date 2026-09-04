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
