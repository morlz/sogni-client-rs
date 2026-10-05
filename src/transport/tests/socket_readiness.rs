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
async fn pre_authentication_disconnects_grow_backoff_until_server_authenticates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = client(listener.local_addr().unwrap(), Duration::from_secs(10));
    let (finish, finished) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut times = Vec::new();
        for cycle in 0..4 {
            let accepted = listener.accept().await.unwrap().0;
            times.push(tokio::time::Instant::now());
            let mut socket = accept_async(accepted).await.unwrap();
            if cycle == 3 {
                let _ = finished.await;
                return times;
            }
            if cycle == 2 {
                authenticate(&mut socket).await;
            }
            socket
                .close(Some(CloseFrame {
                    code: CloseCode::Away,
                    reason: "restart".into(),
                }))
                .await
                .unwrap();
        }
        unreachable!()
    });
    let mut events = client.subscribe();
    client.start().await.unwrap();
    // Do not release the last accepted socket until it is observable. This
    // exercises real upgrades, server-authentication frames and reconnects.
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut connections = 0;
        while connections < 4 {
            if events.recv().await.unwrap().name == "connected" {
                connections += 1;
            }
        }
    })
    .await
    .unwrap();
    let _ = finish.send(());
    let times = server.await.unwrap();
    client.close().await.unwrap();
    let second_delay = times[2] - times[1];
    assert!(
        second_delay >= Duration::from_millis(1500),
        "second pre-authentication disconnect retried too soon: {second_delay:?}"
    );
    let authenticated_delay = times[3] - times[2];
    assert!(
        authenticated_delay < Duration::from_secs(3),
        "authenticated peer did not reset the backoff: {authenticated_delay:?}"
    );
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_sends_share_the_initial_socket_writer() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = std::sync::Arc::new(client(
        listener.local_addr().unwrap(),
        Duration::from_secs(5),
    ));
    let server = tokio::spawn(async move {
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        authenticate(&mut socket).await;
        let mut ids = std::collections::BTreeSet::new();
        while ids.len() < 16 {
            let message = socket.next().await.unwrap().unwrap();
            if let Message::Text(text) = message {
                let frame: Value = serde_json::from_str(&text).unwrap();
                ids.insert(
                    b64_json_decode(frame["data"].as_str().unwrap()).unwrap()["index"]
                        .as_u64()
                        .unwrap(),
                );
            }
        }
        assert_eq!(ids, (0..16).collect());
    });
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut requests = Vec::new();
    for index in 0..16 {
        let client = client.clone();
        let barrier = barrier.clone();
        requests.push(tokio::spawn(async move {
            barrier.wait().await;
            client
                .send_socket("fixtureRequest", &json!({"index":index}))
                .await
        }));
    }
    for request in requests {
        tokio::time::timeout(Duration::from_secs(5), request)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
    server.await.unwrap();
    client.close().await.unwrap();
}

#[tokio::test]
async fn close_cancels_an_unfinished_websocket_handshake_promptly() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = client(listener.local_addr().unwrap(), Duration::from_secs(60));
    client.start().await.unwrap();
    let (_stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), client.close())
        .await
        .unwrap()
        .unwrap();
    assert!(!client.is_authenticated());
    assert!(!client.is_socket_connected());
}
