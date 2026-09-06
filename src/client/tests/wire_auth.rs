use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

const SENTINEL: &str = "fixture-api-key-with-exact-case-ABC123";

#[tokio::test]
async fn full_client_preserves_api_key_on_http_and_websocket_after_initialization() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let checked = Arc::new(AtomicUsize::new(0));
    let captured = checked.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let checked = captured.clone();
            tokio::spawn(async move {
                let mut preview = [0_u8; 8192];
                let header = loop {
                    let length = socket.peek(&mut preview).await.unwrap();
                    if length == 0 {
                        return;
                    }
                    if preview[..length].windows(4).any(|part| part == b"\r\n\r\n") {
                        break String::from_utf8(preview[..length].to_vec()).unwrap();
                    }
                    tokio::task::yield_now().await;
                };
                let key = header.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("api-key").then(|| value.trim())
                });
                assert_eq!(key, Some(SENTINEL));
                checked.fetch_add(1, Ordering::SeqCst);
                if header.to_ascii_lowercase().contains("upgrade: websocket") {
                    let path = header
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap();
                    let url = Url::parse("http://fixture.invalid/")
                        .unwrap()
                        .join(path)
                        .unwrap();
                    let family = url
                        .query_pairs()
                        .find(|(name, _)| name == "clientName")
                        .unwrap()
                        .1;
                    assert_eq!(
                        family,
                        format!("Sogni/3.0.0 (sogni-client) {}", crate::VERSION)
                    );
                    let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
                    let payload = crate::utils::b64_json_encode(&json!({
                        "username":"fixture", "address":"fixture", "subscriptionEntitlement":{}
                    }))
                    .unwrap();
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            json!({
                                "type":"authenticated", "data":payload
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    while let Some(Ok(message)) = socket.next().await {
                        if let tokio_tungstenite::tungstenite::Message::Ping(payload) = message {
                            let _ = socket
                                .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                                .await;
                        }
                    }
                } else {
                    let mut byte = [0_u8; 1];
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        socket.read_exact(&mut byte).await.unwrap();
                        request.extend_from_slice(&byte);
                    }
                    let body = json!({"status":"success", "data":{"username":"fixture","walletAddress":"fixture"},"activeProjects":[],"unclaimedCompletedProjects":[]}).to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                }
            });
        }
    });
    let client = SogniClient::builder()
        .api_key(SENTINEL)
        .rest_endpoint(Url::parse(&format!("http://{address}/")).unwrap())
        .socket_endpoint(Url::parse(&format!("ws://{address}/")).unwrap())
        .request_timeout(Duration::from_secs(2))
        .connect_timeout(Duration::from_secs(2))
        .build()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !client.is_socket_authenticated() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    client.account.me().await.unwrap();
    assert!(client.is_authenticated());
    assert!(client.is_socket_authenticated());
    assert!(checked.load(Ordering::SeqCst) >= 2);
    client.close().await.unwrap();
    server.abort();
}
