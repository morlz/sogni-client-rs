use std::time::Duration;

mod media_timeout;
mod socket_abort;

use futures_util::{SinkExt, StreamExt};
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use url::Url;

use super::{ApiClient, HttpClients, RestClient};
use crate::{
    AuthKind, ClientConfig, Error,
    auth::AuthManager,
    utils::{b64_json_decode, b64_json_encode},
};

#[test]
fn authenticated_rest_paths_cannot_change_origin() {
    let base = Url::parse("https://api.sogni.ai/").expect("base URL");
    let rest = api_key_rest(base.clone(), Duration::from_secs(1));
    assert!(rest.url("/v1/account/me").is_ok());
    assert!(rest.url("https://example.com/collect").is_err());
}

async fn raw_http_fixture(
    status: &str,
    headers: impl FnOnce(&Url) -> String,
    body: &str,
) -> (Url, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let address = listener.local_addr().expect("fixture address");
    let url = Url::parse(&format!("http://{address}/")).expect("fixture URL");
    let status = status.to_owned();
    let headers = headers(&url);
    let body = body.to_owned();
    let (sender, receiver) = oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept fixture request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = socket.read(&mut buffer).await.expect("read request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let _ = sender.send(String::from_utf8_lossy(&request).into_owned());
        let response = format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write fixture response");
    });
    (url, receiver)
}

async fn http_fixture(
    status: &str,
    content_type: &str,
    body: &str,
) -> (Url, oneshot::Receiver<String>) {
    raw_http_fixture(
        status,
        |_: &Url| format!("Content-Type: {content_type}\r\n"),
        body,
    )
    .await
}

async fn redirect_fixture(location: Option<Url>) -> (Url, oneshot::Receiver<String>) {
    raw_http_fixture(
        "302 Found",
        move |base| {
            let location = location.unwrap_or_else(|| base.join("redirected").expect("redirect"));
            format!("Location: {location}\r\nContent-Type: text/plain\r\n")
        },
        "redirect",
    )
    .await
}

fn api_key_auth(base_url: Url, http: &HttpClients) -> AuthManager {
    let auth = AuthManager::new(
        AuthKind::ApiKey,
        base_url,
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("secret-test-key")
        .expect("API key");
    auth
}

pub(super) fn api_key_rest(base_url: Url, timeout: Duration) -> RestClient {
    let http = HttpClients::build(timeout).expect("HTTP clients");
    let auth = api_key_auth(base_url.clone(), &http);
    RestClient::new(base_url, auth, http, timeout)
}

#[tokio::test]
async fn rest_sends_api_key_and_preserves_structured_errors() {
    let (base_url, captured) = http_fixture(
        "429 Too Many Requests",
        "application/json",
        r#"{"message":"slow down","errorCode":1234}"#,
    )
    .await;
    let rest = api_key_rest(base_url, Duration::from_secs(2));
    let error = rest
        .get("/limited", Some(&json!({"tag": ["a", "b"]})))
        .await
        .expect_err("fixture is a 429");
    let Error::Api(error) = error else {
        panic!("expected ApiError");
    };
    assert_eq!(error.status, 429);
    assert_eq!(error.error_code, 1234);
    assert_eq!(error.message, "slow down");
    let request = captured.await.expect("captured request");
    assert!(request.starts_with("GET /limited?tag=a&tag=b HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("api-key: secret-test-key")
    );
}

#[tokio::test]
async fn authenticated_rest_refuses_cross_origin_redirects() {
    let destination = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect destination");
    let destination_url = Url::parse(&format!(
        "http://{}/collect",
        destination.local_addr().expect("destination address")
    ))
    .expect("destination URL");
    let (base_url, captured) = redirect_fixture(Some(destination_url)).await;
    let rest = api_key_rest(base_url, Duration::from_secs(2));

    let error = rest
        .get("/redirect", None)
        .await
        .expect_err("authenticated redirects are refused");
    assert!(matches!(error, Error::Api(ref error) if error.status == 302));
    assert!(
        captured
            .await
            .expect("captured API request")
            .to_ascii_lowercase()
            .contains("api-key: secret-test-key")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), destination.accept())
            .await
            .is_err(),
        "the redirect destination must not receive a request"
    );
}

#[tokio::test]
async fn authenticated_rest_refuses_same_origin_redirects_too() {
    let (base_url, _) = redirect_fixture(None).await;
    let rest = api_key_rest(base_url, Duration::from_secs(2));
    let error = rest
        .get("/redirect", None)
        .await
        .expect_err("API redirects are intentionally refused");
    assert!(matches!(error, Error::Api(ref error) if error.status == 302));
}

#[tokio::test]
async fn media_downloads_follow_redirects_without_credentials() {
    let (destination_url, destination_request) =
        http_fixture("200 OK", "application/octet-stream", "media").await;
    let (base_url, source_request) = redirect_fixture(Some(destination_url)).await;
    let rest = api_key_rest(base_url.clone(), Duration::from_secs(2));

    let bytes = rest
        .get_bytes(base_url.join("asset").expect("media URL"))
        .await
        .expect("redirected media download");
    assert_eq!(bytes.as_ref(), b"media");
    for request in [
        source_request.await.expect("source request"),
        destination_request.await.expect("destination request"),
    ] {
        assert!(!request.to_ascii_lowercase().contains("api-key:"));
    }
}

#[tokio::test]
async fn credential_headers_are_sensitive() {
    let base_url = Url::parse("https://api.sogni.ai/").expect("base URL");
    let http = HttpClients::build(Duration::from_secs(1)).expect("HTTP clients");
    let headers = api_key_auth(base_url, &http)
        .headers()
        .await
        .expect("authentication headers");
    assert!(
        headers
            .get("api-key")
            .expect("api-key header")
            .is_sensitive()
    );
}

#[tokio::test]
async fn sse_parser_handles_multiple_frames_from_transport() {
    let body = "id: 1\nevent: delta\ndata: {\"text\":\"a\"}\n\nid: 2\ndata: done\n\n";
    let (base_url, _) = http_fixture("200 OK", "text/event-stream", body).await;
    let rest = api_key_rest(base_url, Duration::from_secs(2));
    let events = rest
        .stream_sse("/events", None, HeaderMap::new())
        .await
        .expect("open SSE stream")
        .collect::<Vec<_>>()
        .await;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].as_ref().expect("first event").event, "delta");
    assert_eq!(events[0].as_ref().expect("first event").data["text"], "a");
    assert_eq!(events[1].as_ref().expect("second event").data, "done");
}

#[tokio::test]
async fn websocket_uses_base64_envelope_and_normalizes_ids() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WebSocket fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept WebSocket");
        let mut socket = accept_async(socket).await.expect("handshake");
        let message = socket
            .next()
            .await
            .expect("client frame")
            .expect("valid client frame");
        let Message::Text(text) = message else {
            panic!("expected text envelope");
        };
        let envelope: Value = serde_json::from_str(&text).expect("JSON envelope");
        assert_eq!(envelope["type"], "fixtureRequest");
        let decoded = b64_json_decode(envelope["data"].as_str().expect("base64 data"))
            .expect("decode request");
        assert_eq!(decoded["hello"], "world");
        let response = json!({
            "type": "fixtureEvent",
            "data": b64_json_encode(&json!({"jobID": "abc", "imgID": "def"}))
                .expect("encode response"),
        });
        socket
            .send(Message::Text(response.to_string().into()))
            .await
            .expect("send response");
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = socket.close(None).await;
    });

    let rest_endpoint = Url::parse(&format!("http://{address}/")).expect("REST URL");
    let socket_endpoint = Url::parse(&format!("ws://{address}/")).expect("socket URL");
    let config = ClientConfig {
        app_id: "test-app".into(),
        auth_kind: AuthKind::ApiKey,
        rest_endpoint: rest_endpoint.clone(),
        socket_endpoint,
        connect_timeout: Duration::from_secs(3),
        ..ClientConfig::default()
    };
    let http = HttpClients::build(Duration::from_secs(3)).expect("HTTP clients");
    let auth = AuthManager::new(
        AuthKind::ApiKey,
        rest_endpoint,
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("test-key").expect("API key");
    let client = ApiClient::new(config, auth, http).expect("API client");
    let mut events = client.subscribe();
    client
        .send_socket("fixtureRequest", &json!({"hello": "world"}))
        .await
        .expect("send socket message");
    let received = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = events.recv().await.expect("event");
            if event.name == "fixtureEvent" {
                return event.data;
            }
        }
    })
    .await
    .expect("fixture event timeout");
    assert_eq!(received["jobID"], "ABC");
    assert_eq!(received["imgID"], "DEF");
    client.close().await.expect("close client");
    server.await.expect("server task");
}
