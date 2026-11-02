use super::*;

#[tokio::test]
async fn socket_5_72_preserves_mixed_frame_order_and_recovers_after_malformed_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (release, stop) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut socket = accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap();
        let frame = |kind: &str, data: Value| {
            json!({"type":kind,"data":b64_json_encode(&data).unwrap()}).to_string()
        };
        socket
            .send(Message::Text(frame("authenticated", json!({})).into()))
            .await
            .unwrap();
        while let Some(message) = socket.next().await {
            if message.unwrap().is_text() {
                break;
            }
        }
        for index in 0..64 {
            if index % 17 == 0 {
                socket
                    .send(Message::Text("{bad json".into()))
                    .await
                    .unwrap();
                socket
                    .send(Message::Binary(
                        b"{\"type\":\"jobTokens\",\"data\":\"%%%\"}".to_vec().into(),
                    ))
                    .await
                    .unwrap();
            }
            let text = frame(
                "jobTokens",
                json!({"jobID":"job","content":format!("{index}:前進¡")}),
            );
            let message = if index % 2 == 0 {
                Message::Text(text.into())
            } else {
                Message::Binary(text.into_bytes().into())
            };
            socket.send(message).await.unwrap();
        }
        socket
            .send(Message::Binary(
                frame("llmJobResult", json!({"jobID":"job"}))
                    .into_bytes()
                    .into(),
            ))
            .await
            .unwrap();
        let _ = stop.await;
        let _ = socket.close(None).await;
    });
    let endpoint: Url = format!("http://{address}/").parse().unwrap();
    let http = HttpClients::build(Duration::from_secs(5)).unwrap();
    let auth = AuthManager::new(
        AuthKind::ApiKey,
        endpoint.clone(),
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_api_key("synthetic-order-fixture")
        .unwrap();
    let client = ApiClient::new(
        ClientConfig {
            app_id: "receive-order-fixture".into(),
            auth_kind: AuthKind::ApiKey,
            rest_endpoint: endpoint,
            socket_endpoint: format!("ws://{address}/").parse().unwrap(),
            ..Default::default()
        },
        auth,
        http,
    )
    .unwrap();
    let mut events = client.subscribe();
    client
        .send_socket("fixtureRequest", &json!({}))
        .await
        .unwrap();
    let mut chunks = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            match event.name.as_str() {
                "jobTokens" => {
                    assert_eq!(event.data["jobID"], "JOB");
                    chunks.push(event.data["content"].as_str().unwrap().to_owned());
                }
                "llmJobResult" => {
                    assert_eq!(chunks.len(), 64, "terminal event cannot overtake tokens");
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        chunks,
        (0..64).map(|i| format!("{i}:前進¡")).collect::<Vec<_>>()
    );
    let _ = release.send(());
    client.close().await.unwrap();
    server.await.unwrap();
}
