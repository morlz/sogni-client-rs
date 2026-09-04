mod policy;

use std::{collections::VecDeque, sync::Arc, time::Duration};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use rand::Rng as _;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

use self::policy::{DisconnectDisposition, disconnect_disposition, transport_loss};
use super::{
    PROTOCOL_VERSION, RECONNECT_BASE_DELAY, RECONNECT_MAX_DELAY, SWITCH_CONNECTION, SocketCommand,
    SocketInner,
};
use crate::{Error, Network, Result, utils::b64_json_decode};

pub(super) async fn socket_manager(
    inner: Arc<SocketInner>,
    mut commands: mpsc::Receiver<SocketCommand>,
) {
    let mut auth_updates = inner.auth.subscribe();
    let mut reconnect_attempt = 0_u32;
    let mut suspended = false;
    let mut pending = VecDeque::new();
    loop {
        if inner.cancel.is_cancelled() {
            fail_pending(&mut pending, Error::Closed);
            return;
        }
        if !inner.auth.is_authenticated() || suspended {
            tokio::select! {
                () = inner.cancel.cancelled() => {
                    fail_pending(&mut pending, Error::Closed);
                    return;
                }
                changed = auth_updates.changed() => {
                    if changed.is_err() {
                        fail_pending(&mut pending, Error::Closed);
                        return;
                    }
                    if *auth_updates.borrow() {
                        suspended = false;
                    }
                }
                command = commands.recv() => match command {
                    Some(command) if inner.auth.is_authenticated() => {
                        pending.push_back(command);
                        suspended = false;
                    }
                    Some(SocketCommand::Send { response, .. }) => {
                        let _ = response.send(Err(Error::InvalidInput("authentication is required for WebSocket operations".into())));
                    }
                    None => return,
                }
            }
            continue;
        }

        inner.events.emit(
            "connecting",
            json!({"network": inner.network.read().as_str()}),
        );
        match connect_socket(&inner).await {
            Ok(mut socket) => {
                reconnect_attempt = 0;
                inner.connected.send_replace(true);
                inner.events.emit(
                    "connected",
                    json!({"network": inner.network.read().as_str()}),
                );
                while let Some(command) = pending.pop_front() {
                    if let Err(command) = send_command(&mut socket, command).await {
                        pending.push_front(command);
                        break;
                    }
                }
                let outcome = socket_session(&inner, &mut socket, &mut commands).await;
                inner.connected.send_replace(false);
                match outcome {
                    SocketOutcome::Closed => {
                        fail_pending(&mut pending, Error::Closed);
                        return;
                    }
                    SocketOutcome::AuthChanged => {
                        reconnect_attempt = 0;
                    }
                    SocketOutcome::Disconnected { code, reason } => {
                        match disconnect_disposition(code) {
                            DisconnectDisposition::Suspend => {
                                inner
                                    .events
                                    .emit("disconnected", json!({"code": code, "reason": reason}));
                                suspended = true;
                            }
                            DisconnectDisposition::ClearAuthAndSuspend => {
                                inner.auth.clear();
                                inner
                                    .events
                                    .emit("disconnected", json!({"code": code, "reason": reason}));
                                suspended = true;
                            }
                            DisconnectDisposition::Reconnect => {
                                reconnect_attempt = reconnect_attempt.saturating_add(1);
                            }
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "Sogni WebSocket connection attempt failed");
                reconnect_attempt = reconnect_attempt.saturating_add(1);
            }
        }
        if !suspended && inner.auth.is_authenticated() {
            let exponent = reconnect_attempt.saturating_sub(1).min(16) as i32;
            let base = (RECONNECT_BASE_DELAY * 2_f64.powi(exponent)).min(RECONNECT_MAX_DELAY);
            let delay = base * rand::rng().random_range(0.8..1.2);
            tokio::select! {
                () = inner.cancel.cancelled() => return,
                () = tokio::time::sleep(Duration::from_secs_f64(delay)) => {},
            }
        }
    }
}

async fn connect_socket(
    inner: &SocketInner,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let mut url = inner.url.clone();
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("appId", &inner.app_id);
        query.append_pair(
            "clientName",
            &format!(
                "Sogni/{PROTOCOL_VERSION} (sogni-client-rs) {}",
                crate::VERSION
            ),
        );
        query.append_pair("clientType", "artist");
        query.append_pair(
            "forceWorkerId",
            if *inner.network.read() == Network::Fast {
                "fast"
            } else {
                ""
            },
        );
        if let Some(app_source) = &inner.app_source {
            query.append_pair("appSource", app_source);
        }
        for (name, value) in inner.attribution.connection_query() {
            query.append_pair(&name, &value);
        }
        let subscriptions = inner.subscriptions.read();
        if !subscriptions.is_empty() {
            query.append_pair(
                "socketEventSubscriptions",
                &serde_json::to_string(&*subscriptions)?,
            );
        }
    }
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| Error::Transport(format!("failed to build WebSocket request: {error}")))?;
    for (name, value) in inner.auth.headers().await? {
        if let Some(name) = name {
            request.headers_mut().insert(name, value);
        }
    }
    let (socket, _) = tokio::time::timeout(inner.connect_timeout, connect_async(request))
        .await
        .map_err(|_| Error::Timeout("WebSocket connection timed out".into()))?
        .map_err(|error| Error::Transport(format!("WebSocket connection failed: {error}")))?;
    Ok(socket)
}

enum SocketOutcome {
    Closed,
    AuthChanged,
    Disconnected { code: u16, reason: String },
}

async fn socket_session<S>(
    inner: &SocketInner,
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    commands: &mut mpsc::Receiver<SocketCommand>,
) -> SocketOutcome
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut auth_updates = inner.auth.subscribe();
    let mut ping = tokio::time::interval(Duration::from_secs(15));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = inner.cancel.cancelled() => {
                let _ = socket.send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "Client disconnected".into(),
                }))).await;
                return SocketOutcome::Closed;
            }
            changed = auth_updates.changed() => {
                if changed.is_err() || !*auth_updates.borrow() {
                    let _ = socket.send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "Authentication cleared".into(),
                    }))).await;
                    return SocketOutcome::AuthChanged;
                }
            }
            command = commands.recv() => match command {
                Some(command) => {
                    if let Err(command) = send_command(socket, command).await {
                        let SocketCommand::Send { response, .. } = command;
                        let _ = response.send(Err(Error::Transport("WebSocket send failed".into())));
                        return transport_loss("WebSocket send failed");
                    }
                }
                None => return SocketOutcome::Closed,
            },
            _ = ping.tick() => {
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    return transport_loss("WebSocket ping failed");
                }
            }
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Text(text))) => handle_socket_frame(inner, text.as_bytes()),
                Some(Ok(Message::Binary(bytes))) => handle_socket_frame(inner, &bytes),
                Some(Ok(Message::Ping(payload))) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        return transport_loss("WebSocket pong failed");
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    let (code, reason) = frame.map_or((1000, String::new()), |frame| (u16::from(frame.code), frame.reason.to_string()));
                    return SocketOutcome::Disconnected { code, reason };
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    tracing::warn!(error = %error, "Sogni WebSocket receive failed");
                    return transport_loss(error.to_string());
                }
                None => return transport_loss("WebSocket stream ended"),
            }
        }
    }
}

async fn send_command<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    command: SocketCommand,
) -> std::result::Result<(), SocketCommand>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match command {
        SocketCommand::Send { message, response } => {
            if response.is_closed() {
                return Ok(());
            }
            if let Err(error) = socket.send(message.clone()).await {
                tracing::warn!(error = %error, "Sogni WebSocket send failed");
                return Err(SocketCommand::Send { message, response });
            }
            let _ = response.send(Ok(()));
            Ok(())
        }
    }
}

fn handle_socket_frame(inner: &SocketInner, bytes: &[u8]) {
    let parsed = (|| -> Result<(String, Value)> {
        let envelope: Value = serde_json::from_slice(bytes)?;
        let message_type = envelope
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("WebSocket envelope has no string type".into()))?
            .to_owned();
        let mut payload = envelope
            .get("data")
            .and_then(Value::as_str)
            .map(b64_json_decode)
            .transpose()?
            .unwrap_or(Value::Null);
        if let Some(payload) = payload.as_object_mut() {
            for key in ["jobID", "imgID"] {
                if let Some(value) = payload.get_mut(key) {
                    if let Some(text) = value.as_str() {
                        *value = Value::String(text.to_uppercase());
                    }
                }
            }
            if message_type == "socketEventSubscriptionsUpdated" {
                if let Some(subscriptions) = payload
                    .get("socketEventSubscriptions")
                    .and_then(Value::as_object)
                {
                    *inner.subscriptions.write() = subscriptions
                        .iter()
                        .filter_map(|(key, value)| {
                            value.as_bool().map(|value| (key.clone(), value))
                        })
                        .collect();
                }
            }
        }
        Ok((message_type, payload))
    })();
    match parsed {
        Ok((message_type, payload)) => inner.events.emit(message_type, payload),
        Err(error) => tracing::warn!(error = %error, "dropped malformed Sogni WebSocket frame"),
    }
}

fn fail_pending(pending: &mut VecDeque<SocketCommand>, error: Error) {
    let message = error.to_string();
    while let Some(SocketCommand::Send { response, .. }) = pending.pop_front() {
        let _ = response.send(Err(Error::Transport(message.clone())));
    }
}

#[cfg(test)]
mod tests;
