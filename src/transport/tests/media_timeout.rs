use super::*;

#[tokio::test]
async fn media_transfers_honor_configured_request_timeout() {
    for method in ["put", "multipart", "get"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut input = [0_u8; 1024];
            let _ = socket.read(&mut input).await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        });
        let url = Url::parse(&format!("http://{address}/")).unwrap();
        let rest = api_key_rest(url.clone(), Duration::from_millis(30));
        let operation = async {
            match method {
                "put" => {
                    rest.put_bytes(url, bytes::Bytes::from_static(b"fixture"), None)
                        .await
                }
                "multipart" => {
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
        let result = tokio::time::timeout(Duration::from_millis(250), operation)
            .await
            .expect("configured timeout must apply before server response");
        assert!(match result {
            Err(Error::Http(error)) => error.is_timeout(),
            Err(Error::Timeout(_)) => method == "put",
            _ => false,
        });
        server.abort();
    }
}
