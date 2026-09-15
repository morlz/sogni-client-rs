use super::*;

#[tokio::test]
async fn media_transfers_honor_configured_request_timeout() {
    for method in [
        "put_bytes",
        "put_saved_asset",
        "post_multipart",
        "get_bytes",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut input = [0_u8; 1024];
            let _ = socket.read(&mut input).await;
            // Keep the connection open until the harness aborts this fixture.
            // A delayed close would race the SDK deadline on a busy runner.
            std::future::pending::<()>().await;
            drop(socket);
        });
        let url = Url::parse(&format!("http://{address}/")).unwrap();
        let rest = api_key_rest(url.clone(), Duration::from_millis(30));
        let operation = async {
            match method {
                "put_bytes" => {
                    rest.put_bytes(url, bytes::Bytes::from_static(b"fixture"), None)
                        .await
                }
                "put_saved_asset" => {
                    let mut headers = HeaderMap::new();
                    headers.insert("if-none-match", "*".parse().unwrap());
                    rest.put_saved_asset(url, bytes::Bytes::from_static(b"fixture"), headers)
                        .await
                }
                "post_multipart" => {
                    rest.post_multipart(
                        url,
                        &Default::default(),
                        bytes::Bytes::from_static(b"fixture"),
                        "fixture.bin",
                        None,
                    )
                    .await
                }
                _ => rest.get_bytes(url).await.map(|_| ()),
            }
        };
        // Allow client construction and OS scheduling to exceed the request's
        // 30 ms budget, while still catching a saved PUT's fixed 300 s timeout.
        // The fixture never responds, so only an SDK timeout can pass below.
        let result = tokio::time::timeout(Duration::from_secs(10), operation).await;
        server.abort();
        if let Err(error) = server.await {
            assert!(error.is_cancelled(), "{method}: fixture failed: {error}");
        }
        let result = result.unwrap_or_else(|_| {
            panic!("{method}: configured 30 ms request timeout did not finish within 10 s")
        });
        assert!(
            match &result {
                Err(Error::Http(error)) => error.is_timeout(),
                Err(Error::Timeout(_)) => method == "put_bytes",
                _ => false,
            },
            "{method}: expected the configured request timeout, got {result:?}"
        );
    }
}
