use super::*;

#[tokio::test]
async fn invalid_5_69_requests_fail_before_any_upload_or_socket_connection() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let client = crate::SogniClient::builder()
        .rest_endpoint(format!("http://{address}").parse().unwrap())
        .socket_endpoint(format!("ws://{address}").parse().unwrap())
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let seedream = crate::SEEDREAM_5_PRO_UNCENSORED_MODEL_ID;
    let h3 = "minimax-h3-fastvideo-int8_t2v_turbo";
    *client.projects.inner.supported_models.write() = Some(TimedValue {
        value: json!([{"id":seedream,"tier":"image"},{"id":h3,"tier":"video"}]),
        loaded_at: Instant::now(),
    });
    *client.projects.inner.model_tiers.write() = Some(TimedValue {
        value: json!({"image":{"type":"image"},"video":{"type":"video"}}),
        loaded_at: Instant::now(),
    });
    let requests = [
        ProjectRequest::image(seedream, "a portrait")
            .param("width", 1024)
            .param("height", 768)
            .asset(
                AssetRole::ContextImage(1),
                MediaSource::named_bytes(vec![1, 2, 3], "reference.png", "image/png"),
            ),
        ProjectRequest::image(seedream, "an edit").asset(
            AssetRole::ContextImage(11),
            MediaSource::named_bytes(vec![1, 2, 3], "reference.png", "image/png"),
        ),
        ProjectRequest::video(h3, "continue")
            .param("frames", 141)
            .extend_output("both")
            .asset(
                AssetRole::ReferenceVideo,
                MediaSource::named_bytes(vec![1, 2, 3], "source.mp4", "video/mp4"),
            ),
    ];
    for (request, expected) in requests.into_iter().zip(["921,600", "10", "extendOutput"]) {
        let result = tokio::time::timeout(Duration::from_secs(1), client.projects.create(request))
            .await
            .expect("invalid input must not wait for network I/O");
        let message = match result {
            Err(Error::InvalidInput(message)) => message,
            Err(error) => panic!("expected input validation: {error}"),
            Ok(_) => panic!("invalid input was submitted"),
        };
        assert!(message.contains(expected), "{message}");
        assert!(client.projects.inner.projects.read().is_empty());
    }
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
    client.close().await.unwrap();
}
