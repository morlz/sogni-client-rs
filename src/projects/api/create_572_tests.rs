use super::*;

#[tokio::test]
async fn compression_5_72_refusals_precede_uploads_and_project_tracking() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let client = crate::SogniClient::builder()
        .disable_socket(true)
        .rest_endpoint(format!("http://{address}").parse().unwrap())
        .socket_endpoint(format!("ws://{address}").parse().unwrap())
        .build()
        .await
        .unwrap();
    let one = "minimax-h3-ref2va-fp8_r2v_balanced";
    let two = "minimax-h3-ref2va-fp8_r2v_balanced_2stage";
    *client.projects.inner.supported_models.write() = Some(TimedValue {
        value: json!([{"id":one,"tier":"video"},{"id":two,"tier":"video"}]),
        loaded_at: Instant::now(),
    });
    *client.projects.inner.model_tiers.write() = Some(TimedValue {
        value: json!({"video":{"type":"video"}}),
        loaded_at: Instant::now(),
    });
    for (model, mode, role) in [
        (two, "vpack", AssetRole::ReferenceVideo),
        (one, "vpack", AssetRole::ReferenceImage),
        (one, "automatic", AssetRole::ReferenceVideo),
    ] {
        let image = matches!(role, AssetRole::ReferenceImage);
        let source = MediaSource::named_bytes(
            vec![1, 2, 3],
            if image { "ref.png" } else { "ref.mp4" },
            if image { "image/png" } else { "video/mp4" },
        );
        let request = ProjectRequest::video(model, "A character walks.")
            .param("frames", 124)
            .param("steps", 8)
            .param("width", 1344)
            .param("height", 768)
            .reference_video_compression(mode)
            .asset(role, source);
        let result = tokio::time::timeout(Duration::from_secs(2), client.projects.create(request))
            .await
            .unwrap();
        let Error::InvalidInput(message) = result.unwrap_err() else {
            panic!("expected pre-upload input validation")
        };
        assert!(
            message.contains(if mode == "automatic" {
                "referenceVideoCompression"
            } else {
                "VPack"
            }),
            "{message}"
        );
        assert!(client.projects.inner.projects.read().is_empty());
    }
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
    client.close().await.unwrap();
}
