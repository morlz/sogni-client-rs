use super::*;
use crate::{
    SogniClient,
    utils::{b64_json_decode, b64_json_encode},
};
use axum::{
    Json, Router,
    http::HeaderMap,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::oneshot};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[tokio::test]
async fn automatic_media_advertises_ciphertext_only_after_one_account_key_fetch() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let rest = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rest_url = format!("http://{}", rest.local_addr().unwrap()).parse().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let key_requests = calls.clone();
        let app = Router::new()
            .route("/v1/account/media-key", get(move |headers: HeaderMap| {
                let key_requests = key_requests.clone();
                async move {
                    assert_eq!(headers["api-key"], "fixture");
                    assert!(!headers.contains_key("x-sogni-media"));
                    key_requests.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"data":{"version":1,"current":{"id":"synthetic","key":STANDARD.encode([9u8;32])}}}))
                }
            }))
            .route("/v1/account/me", get(|headers: HeaderMap| async move {
                Json(json!({"data":{"username":"fixture","capability":headers.get("x-sogni-media").and_then(|h|h.to_str().ok())}}))
            }))
            .route("/v1/account/logout", post(|| async { Json(json!({})) }));
        let http = tokio::spawn(async move { axum::serve(rest, app).await.unwrap(); });
        let websocket = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket_url = format!("ws://{}", websocket.local_addr().unwrap()).parse().unwrap();
        let (initialize, ready) = oneshot::channel();
        let (observed, advertised) = oneshot::channel();
        let peer = tokio::spawn(async move {
            let mut ws = accept_async(websocket.accept().await.unwrap().0).await.unwrap();
            ready.await.unwrap();
            let data = b64_json_encode(&json!({"mediaEncryptionAvailable":true})).unwrap();
            ws.send(Message::Text(json!({"type":"authenticated","data":data}).to_string().into())).await.unwrap();
            loop {
                if let Message::Text(text) = ws.next().await.unwrap().unwrap() {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(frame["type"], "mediaCapability");
                    let data = b64_json_decode(frame["data"].as_str().unwrap()).unwrap();
                    assert_eq!(data, json!({"sogniMedia":"ciphertext-v1"}));
                    observed.send(()).unwrap();
                    break;
                }
            }
            while let Some(Ok(message)) = ws.next().await {
                assert!(!message.is_text());
            }
        });
        let client = SogniClient::builder().app_id("encrypted-fixture").api_key("fixture")
            .rest_endpoint(rest_url).socket_endpoint(socket_url)
            .media_encryption(MediaEncryptionMode::Auto).build().await.unwrap();
        assert!(!client.media_encryption.available());
        assert!(!client.media_encryption.is_unlocked().await);
        initialize.send(()).unwrap();
        advertised.await.unwrap();
        let (a, b) = tokio::join!(client.media_encryption.is_unlocked(), client.media_encryption.is_unlocked());
        assert!(a && b && client.media_encryption.available());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(client.account.me().await.unwrap()["capability"], "ciphertext-v1");
        client.account.logout().await.unwrap();
        assert!(!client.media_encryption.available());
        assert!(!client.media_encryption.is_unlocked().await);
        client.close().await.unwrap();
        peer.await.unwrap();
        http.abort();
    }).await.expect("automatic encryption fixture should finish");
}
