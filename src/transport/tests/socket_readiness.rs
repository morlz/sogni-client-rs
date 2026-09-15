use super::*;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::protocol::{CloseFrame, frame::coding::CloseCode},
};

fn client(address: std::net::SocketAddr, timeout: Duration) -> ApiClient {
    let endpoint: Url = format!("http://{address}/").parse().unwrap();
    let http = HttpClients::build(Duration::from_secs(2)).unwrap();
    let auth = AuthManager::new(
        AuthKind::ApiKey,
        endpoint.clone(),
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("fixture-key").unwrap();
    ApiClient::new(
        ClientConfig {
            app_id: "readiness-fixture".into(),
            auth_kind: AuthKind::ApiKey,
            rest_endpoint: endpoint,
            socket_endpoint: format!("ws://{address}/").parse().unwrap(),
            connect_timeout: timeout,
            ..Default::default()
        },
        auth,
        http,
    )
    .unwrap()
}

async fn assert_no_work(socket: &mut WebSocketStream<tokio::net::TcpStream>, duration: Duration) {
    let _ = tokio::time::timeout(duration, async {
        while let Some(message) = socket.next().await {
            assert!(
                !message.unwrap().is_text(),
                "work reached a socket before readiness or after send timeout"
            );
        }
    })
    .await;
}

async fn authenticate(socket: &mut WebSocketStream<tokio::net::TcpStream>) {
    socket
        .send(Message::Text(
            json!({"type":"authenticated", "data":b64_json_encode(&json!({})).unwrap()})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn send_waits_through_a_restart_and_the_new_authentication_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = client(listener.local_addr().unwrap(), Duration::from_secs(6));
    let server = tokio::spawn(async move {
        let mut first = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        assert_no_work(&mut first, Duration::from_millis(40)).await;
        first
            .close(Some(CloseFrame {
                code: CloseCode::Away,
                reason: "restart".into(),
            }))
            .await
            .unwrap();
        let mut second = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        assert_no_work(&mut second, Duration::from_millis(80)).await;
        authenticate(&mut second).await;
        while let Some(message) = second.next().await {
            let message = message.unwrap();
            if let Message::Text(text) = message {
                let envelope: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(envelope["type"], "jobRequest");
                assert_eq!(
                    b64_json_decode(envelope["data"].as_str().unwrap()).unwrap()["jobID"],
                    "PROJECT"
                );
                return;
            }
        }
        panic!("missing request");
    });
    client
        .send_socket("jobRequest", &json!({"jobID":"PROJECT"}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    client.close().await.unwrap();
}

#[tokio::test]
async fn expired_queued_sends_are_not_delivered_after_authentication() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = client(listener.local_addr().unwrap(), Duration::from_millis(40));
    let server = tokio::spawn(async move {
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        assert_no_work(&mut socket, Duration::from_millis(100)).await;
        authenticate(&mut socket).await;
        assert_no_work(&mut socket, Duration::from_millis(100)).await;
    });
    assert!(matches!(
        client.send_socket("jobRequest", &json!({})).await,
        Err(Error::Timeout(_))
    ));
    server.await.unwrap();
    client.close().await.unwrap();
}

#[tokio::test]
async fn terminal_close_ends_wait_for_readiness() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = client(listener.local_addr().unwrap(), Duration::from_secs(3));
    let server = tokio::spawn(async move {
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        assert_no_work(&mut socket, Duration::from_millis(40)).await;
        socket
            .close(Some(CloseFrame {
                code: CloseCode::from(4021),
                reason: "not authenticated".into(),
            }))
            .await
            .unwrap();
    });
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        client.send_socket("jobRequest", &json!({})),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    server.await.unwrap();
    client.close().await.unwrap();
}
