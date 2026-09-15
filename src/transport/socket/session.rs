mod policy;
mod tls;

use std::{
    collections::VecDeque,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use rand::Rng as _;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    client_async_tls_with_config, connect_async_tls_with_config,
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
use crate::{Error, Network, Result, auth::AuthVersion, utils::b64_json_decode};

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
            Ok((mut socket, version)) => {
                if version.session != inner.auth.version().session {
                    let _ = socket.close(None).await;
                    continue;
                }
                reconnect_attempt = 0;
                inner.session.store(version.session, Ordering::Release);
                inner.authenticated.store(false, Ordering::Release);
                inner.connected.send_replace(true);
                inner.events.emit(
                    "connected",
                    json!({"network": inner.network.read().as_str()}),
                );
                let outcome = socket_session(
                    &inner,
                    &mut socket,
                    &mut commands,
                    &mut pending,
                    version.session,
                )
                .await;
                inner.connected.send_replace(false);
                inner.authenticated.store(false, Ordering::Release);
                match outcome {
                    SocketOutcome::Closed => {
                        fail_pending(&mut pending, Error::Closed);
                        inner.events.emit(
                            "disconnected",
                            json!({"code":1000,"reason":"Client disconnected"}),
                        );
                        return;
                    }
                    SocketOutcome::AuthChanged => {
                        discard_stale(&mut pending, inner.auth.version().session);
                        inner.events.emit(
                            "disconnected",
                            json!({"code":0,"reason":"Authentication changed"}),
                        );
                        reconnect_attempt = 0;
                        continue;
                    }
                    SocketOutcome::Disconnected { code, reason } => {
                        match disconnect_disposition(code) {
                            DisconnectDisposition::Suspend => {
                                fail_pending(&mut pending, Error::Closed);
                                fail_queued(&mut commands);
                                inner
                                    .events
                                    .emit("disconnected", json!({"code": code, "reason": reason}));
                                suspended = true;
                            }
                            DisconnectDisposition::ClearAuthAndSuspend => {
                                fail_pending(&mut pending, Error::Closed);
                                fail_queued(&mut commands);
                                inner.auth.clear_if_version(version);
                                inner
                                    .events
                                    .emit("disconnected", json!({"code": code, "reason": reason}));
                                suspended = true;
                            }
                            DisconnectDisposition::Reconnect => {
                                inner.events.emit(
                                    "connecting",
                                    json!({"network":inner.network.read().as_str()}),
                                );
                                reconnect_attempt = reconnect_attempt.saturating_add(1);
                            }
                        }
                    }
                }
            }
            Err(_) => {
                tracing::warn!("Sogni WebSocket connection attempt failed");
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
) -> Result<(
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    AuthVersion,
)> {
    let mut url = inner.url.clone();
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("appId", &inner.app_id);
        query.append_pair(
            "clientName",
            &format!(
                // The service's API-key handshake requires this protocol family.
                // HTTP User-Agent keeps the actual sogni-client-rs identity.
                "Sogni/{PROTOCOL_VERSION} (sogni-client) {}",
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
    let (version, mut headers) = inner.auth.headers().await?;
    if let Some(cookie) = inner.auth.socket_cookie(&url) {
        headers.insert(reqwest::header::COOKIE, cookie);
    }
    for (name, value) in headers {
        if let Some(name) = name {
            request.headers_mut().insert(name, value);
        }
    }
    let connector = if url.scheme() == "wss" {
        Some(tls::native_connector()?)
    } else {
        None
    };
    let (socket, _) = tokio::time::timeout(inner.connect_timeout, async {
        if let Some(proxy) = &inner.proxy {
            let stream = proxy.connect(&url).await?;
            client_async_tls_with_config(request, stream, None, connector)
                .await
                .map_err(|_| Error::Transport("WebSocket proxy handshake failed".into()))
        } else {
            connect_async_tls_with_config(request, None, false, connector)
                .await
                .map_err(|_| Error::Transport("WebSocket handshake failed".into()))
        }
    })
    .await
    .map_err(|_| Error::Timeout("WebSocket connection timed out".into()))?
    .map_err(|error| Error::Transport(format!("WebSocket connection failed: {error}")))?;
    Ok((socket, version))
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
    pending: &mut VecDeque<SocketCommand>,
    session: u64,
) -> SocketOutcome
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut auth_updates = inner.auth.subscribe_session();
    let fallback_at = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut legacy_ready = false;
    let mut ping = tokio::time::interval(Duration::from_secs(15));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        if inner.auth.version().session != session {
            let _ = socket.close(None).await;
            return SocketOutcome::AuthChanged;
        }
        if inner.authenticated.load(Ordering::Acquire) || legacy_ready {
            while let Some(command) = pending.pop_front() {
                if inner.auth.version().session != session {
                    pending.push_front(command);
                    let _ = socket.close(None).await;
                    return SocketOutcome::AuthChanged;
                }
                if let Err(SocketCommand::Send { response, .. }) =
                    send_command(socket, command, session).await
                {
                    // A failed write may have reached the peer; only definitely
                    // unsent queued work survives reconnect. Never replay it here.
                    let _ = response.send(Err(Error::Transport("WebSocket send failed".into())));
                    return transport_loss("WebSocket send failed");
                }
            }
        }
        tokio::select! {
            () = inner.cancel.cancelled() => {
                let _ = socket.send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "Client disconnected".into(),
                }))).await;
                return SocketOutcome::Closed;
            }
            changed = auth_updates.changed() => {
                if changed.is_err() || *auth_updates.borrow() != session {
                    let _ = socket.send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "Authentication changed".into(),
                    }))).await;
                    return SocketOutcome::AuthChanged;
                }
            }
            command = commands.recv() => match command {
                Some(command) => {
                    pending.push_back(command);
                }
                None => return SocketOutcome::Closed,
            },
            () = tokio::time::sleep_until(fallback_at), if !legacy_ready => {
                legacy_ready = true;
            },
            _ = ping.tick() => {
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    return transport_loss("WebSocket ping failed");
                }
            }
            incoming = socket.next() => {
                if inner.auth.version().session != session {
                    let _ = socket.close(None).await;
                    return SocketOutcome::AuthChanged;
                }
                match incoming {
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
                    tracing::warn!("Sogni WebSocket receive failed");
                    return transport_loss(error.to_string());
                }
                None => return transport_loss("WebSocket stream ended"),
                }
            }
        }
    }
}

async fn send_command<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    command: SocketCommand,
    connection_session: u64,
) -> std::result::Result<(), SocketCommand>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match command {
        SocketCommand::Send {
            message,
            response,
            session,
        } => {
            if response.is_closed() {
                return Ok(());
            }
            if session != connection_session {
                let _ = response.send(Err(Error::InvalidInput(
                    "account session changed before send".into(),
                )));
                return Ok(());
            }
            if socket.send(message.clone()).await.is_err() {
                tracing::warn!("Sogni WebSocket send failed");
                return Err(SocketCommand::Send {
                    message,
                    response,
                    session,
                });
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
        Ok((message_type, payload)) => {
            if message_type == "authenticated" && payload.is_object() {
                inner.authenticated.store(true, Ordering::Release);
            }
            inner.events.emit(message_type, payload);
        }
        Err(_) => tracing::warn!("dropped malformed Sogni WebSocket frame"),
    }
}

fn fail_pending(pending: &mut VecDeque<SocketCommand>, error: Error) {
    let message = error.to_string();
    while let Some(SocketCommand::Send { response, .. }) = pending.pop_front() {
        let _ = response.send(Err(Error::Transport(message.clone())));
    }
}

fn discard_stale(pending: &mut VecDeque<SocketCommand>, current: u64) {
    let mut retained = VecDeque::new();
    while let Some(command) = pending.pop_front() {
        let SocketCommand::Send { session, .. } = &command;
        if *session == current {
            retained.push_back(command);
        } else {
            let SocketCommand::Send { response, .. } = command;
            let _ = response.send(Err(Error::Closed));
        }
    }
    *pending = retained;
}

fn fail_queued(commands: &mut mpsc::Receiver<SocketCommand>) {
    while let Ok(SocketCommand::Send { response, .. }) = commands.try_recv() {
        let _ = response.send(Err(Error::Closed));
    }
}

#[cfg(test)]
mod tests;
