use std::sync::Arc;

use super::*;

#[tokio::test]
async fn abort_stops_queued_sends_and_closes_all_client_clones() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted, handshake_started) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let _ = accepted.send(());
        let _ = released.await;
        let outcome = tokio::time::timeout(Duration::from_secs(1), accept_async(socket)).await;
        if let Ok(Ok(mut socket)) = outcome {
            while let Some(Ok(message)) = socket.next().await {
                assert!(
                    !message.is_text(),
                    "aborted queued request must not be sent"
                );
            }
        }
    });
    let rest_endpoint = Url::parse(&format!("http://{address}/")).unwrap();
    let config = ClientConfig {
        app_id: "abort-fixture".into(),
        auth_kind: AuthKind::ApiKey,
        rest_endpoint: rest_endpoint.clone(),
        socket_endpoint: Url::parse(&format!("ws://{address}/")).unwrap(),
        connect_timeout: Duration::from_secs(2),
        ..ClientConfig::default()
    };
    let http = HttpClients::build(Duration::from_secs(2)).unwrap();
    let auth = AuthManager::new(
        AuthKind::ApiKey,
        rest_endpoint,
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("fixture-key").unwrap();
    let client = Arc::new(ApiClient::new(config, auth, http).unwrap());
    let sending = client.clone();
    let submission =
        tokio::spawn(async move { sending.send_socket("fixtureRequest", &json!({})).await });
    handshake_started.await.unwrap();
    client.abort();
    assert!(!client.is_authenticated());
    assert!(!client.is_socket_connected());
    let _ = release.send(());
    assert!(
        tokio::time::timeout(Duration::from_secs(1), submission)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(matches!(client.start().await, Err(Error::Closed)));
    server.await.unwrap();
}

#[tokio::test]
async fn websocket_upgrade_is_not_server_authentication() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (release, ready) = oneshot::channel();
    let (finish, finished) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        ready.await.unwrap();
        socket.send(Message::Text(json!({
            "type":"authenticated", "data":b64_json_encode(&json!({"fixture":true})).unwrap()
        }).to_string().into())).await.unwrap();
        let _ = finished.await;
    });
    let rest_endpoint = Url::parse(&format!("http://{address}/")).unwrap();
    let http = HttpClients::build(Duration::from_secs(2)).unwrap();
    let auth = AuthManager::new(
        AuthKind::ApiKey,
        rest_endpoint.clone(),
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("fixture-key").unwrap();
    let client = ApiClient::new(
        ClientConfig {
            app_id: "authenticated-fixture".into(),
            auth_kind: AuthKind::ApiKey,
            rest_endpoint,
            socket_endpoint: Url::parse(&format!("ws://{address}/")).unwrap(),
            connect_timeout: Duration::from_secs(2),
            ..Default::default()
        },
        auth,
        http,
    )
    .unwrap();
    client.start().await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !client.is_socket_connected() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!client.is_socket_authenticated());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !client.is_socket_authenticated() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    client.abort();
    assert!(!client.is_socket_authenticated());
    let _ = finish.send(());
    server.await.unwrap();
}
