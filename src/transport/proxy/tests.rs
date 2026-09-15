use std::time::Duration;

use futures_util::StreamExt;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};

use super::*;
use crate::{
    AuthKind, ClientConfig,
    auth::AuthManager,
    transport::{ApiClient, HttpClients},
};

// The callback trait fixes the unboxed error response type for the handshake.
struct CheckApiKey;

impl Callback for CheckApiKey {
    fn on_request(
        self,
        request: &Request,
        response: Response,
    ) -> std::result::Result<Response, ErrorResponse> {
        assert_eq!(request.headers().get("api-key").unwrap(), "fixture-key");
        Ok(response)
    }
}

#[test]
fn proxy_validation_and_debug_never_expose_credentials() {
    for value in [
        "http://localhost:1080",
        "socks5://localhost:0",
        "socks5://localhost/path",
        "socks5://localhost?secret=value",
    ] {
        assert!(SocksProxy::parse(value).is_err());
    }
    let config = ClientConfig {
        proxy_url: Some("socks5h://secret-user:secret-password@localhost:1080".into()),
        ..Default::default()
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains("secret-user"));
    assert!(!debug.contains("secret-password"));
}

#[tokio::test]
async fn proxy_dns_mode_keeps_remote_names_and_local_addresses_distinct() {
    let remote = SocksProxy::parse("socks5h://127.0.0.1:1080").unwrap();
    let destination = Url::parse("https://fixture.invalid/").unwrap();
    assert!(
        matches!(remote.target(&destination).await.unwrap(), TargetAddr::Domain(name, 443) if name == "fixture.invalid")
    );
    let local = SocksProxy::parse("socks5://127.0.0.1:1080").unwrap();
    let destination = Url::parse("https://127.0.0.1:443/").unwrap();
    assert!(matches!(
        local.target(&destination).await.unwrap(),
        TargetAddr::Ip(_)
    ));
}

#[tokio::test]
async fn explicit_proxy_routes_authenticated_rest_media_and_websocket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("socks5h://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for index in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut greeting = [0_u8; 2];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting[0], 5);
            let mut methods = vec![0_u8; usize::from(greeting[1])];
            socket.read_exact(&mut methods).await.unwrap();
            assert!(methods.contains(&0));
            socket.write_all(&[5, 0]).await.unwrap();
            let mut connect = [0_u8; 5];
            socket.read_exact(&mut connect).await.unwrap();
            assert_eq!(&connect[..4], &[5, 1, 0, 3]);
            let mut host = vec![0_u8; usize::from(connect[4])];
            socket.read_exact(&mut host).await.unwrap();
            assert_eq!(host, b"fixture.invalid");
            let mut port = [0_u8; 2];
            socket.read_exact(&mut port).await.unwrap();
            assert_eq!(u16::from_be_bytes(port), 80);
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .await
                .unwrap();
            if index == 2 {
                let mut socket = tokio_tungstenite::accept_hdr_async(socket, CheckApiKey)
                    .await
                    .unwrap();
                futures_util::SinkExt::send(&mut socket, tokio_tungstenite::tungstenite::Message::Text(
                    json!({"type":"authenticated","data":crate::utils::b64_json_encode(&json!({})).unwrap()})
                        .to_string().into())).await.unwrap();
                while let Some(Ok(message)) = socket.next().await {
                    if message.is_text() {
                        assert!(message.into_text().unwrap().contains("fixtureRequest"));
                        break;
                    }
                }
            } else {
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while !request.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).await.unwrap();
                    request.extend_from_slice(&byte);
                    assert!(request.len() < 8192);
                }
                let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                assert!(request.contains("host: fixture.invalid"));
                assert_eq!(request.contains("api-key: fixture-key"), index == 0);
                if index == 1 {
                    let mut body = [0_u8; 7];
                    socket.read_exact(&mut body).await.unwrap();
                    assert_eq!(&body, b"fixture");
                }
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .await
                    .unwrap();
            }
        }
    });
    let endpoint = Url::parse("http://fixture.invalid/").unwrap();
    let http = HttpClients::build_with_proxy(Duration::from_secs(2), &proxy_url).unwrap();
    let auth = AuthManager::new(
        AuthKind::ApiKey,
        endpoint.clone(),
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("fixture-key").unwrap();
    let client = ApiClient::new(
        ClientConfig {
            app_id: "socks-fixture".into(),
            auth_kind: AuthKind::ApiKey,
            rest_endpoint: endpoint.clone(),
            socket_endpoint: Url::parse("ws://fixture.invalid/").unwrap(),
            connect_timeout: Duration::from_secs(2),
            proxy_url: Some(proxy_url),
            ..Default::default()
        },
        auth,
        http,
    )
    .unwrap();
    client.rest.get("/", None).await.unwrap();
    client
        .rest
        .put_bytes(endpoint, bytes::Bytes::from_static(b"fixture"), None)
        .await
        .unwrap();
    client
        .send_socket("fixtureRequest", &json!({}))
        .await
        .unwrap();
    client.close().await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}
