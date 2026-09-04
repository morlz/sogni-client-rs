use super::{SWITCH_CONNECTION, SocketOutcome};

pub(super) const ABNORMAL_CLOSURE: u16 = 1006;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DisconnectDisposition {
    Suspend,
    ClearAuthAndSuspend,
    Reconnect,
}

pub(super) fn disconnect_disposition(code: u16) -> DisconnectDisposition {
    if code == 1000 || code == SWITCH_CONNECTION {
        DisconnectDisposition::Suspend
    } else if code == 0 || (4000..5000).contains(&code) {
        DisconnectDisposition::ClearAuthAndSuspend
    } else {
        DisconnectDisposition::Reconnect
    }
}

pub(super) fn transport_loss(reason: impl Into<String>) -> SocketOutcome {
    SocketOutcome::Disconnected {
        code: ABNORMAL_CLOSURE,
        reason: reason.into(),
    }
}
