use super::*;
use crate::SogniClient;
use axum::{Json, Router, extract::Query, routing::get};

#[tokio::test]
async fn live_and_recovered_receipts_are_normalized_without_exposing_unknown_fields() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let project = Project::new(
        "PROJECT".into(),
        json!({"type":"image","modelId":"sam3_image_segment_bf16","numberOfMedia":1}),
        false,
        Arc::downgrade(&client.projects.inner),
    );
    client
        .projects
        .inner
        .projects
        .write()
        .insert(project.id(), project.clone());
    let mut events = project.subscribe();
    handle_job_result(
        &client.projects.inner,
        &json!({
            "jobID":"PROJECT","imgID":"MASK", "resultUrl":"https://example.test/mask.png",
            "sha256":"A".repeat(64), "sourceImageSha256":"B".repeat(64),
            "maskRleSha256":"invalid", "maskWidth":1024.0,"maskHeight":0,
            "samVersion":"sam3-test", "selectionHash":"C".repeat(64),
            "result":{"samPromptSha256":"D".repeat(64),"internalField":"not public"},
        }),
    )
    .await;
    let job = project.job("MASK").unwrap();
    let expected = json!({
        "sha256":"a".repeat(64),"sourceImageSha256":"b".repeat(64),
        "samPromptSha256":"d".repeat(64),"maskWidth":1024,
        "samVersion":"sam3-test","selectionHash":"c".repeat(64),
    });
    assert_eq!(json!(job.provenance()), expected);
    loop {
        let event = events.recv().await.unwrap();
        if event.name == "jobCompleted" {
            assert_eq!(event.data["provenance"], expected);
            break;
        }
    }
    let recovered = Project::new(
        "RECOVERED".into(),
        json!({"type":"image"}),
        true,
        Arc::downgrade(&client.projects.inner),
    );
    replay_recovered(
        &recovered,
        &json!({
            "status":"completed","completedWorkerJobs":[{
                "id":"MASK","status":"jobCompleted", "result":expected,
            }]
        }),
        true,
    );
    assert_eq!(json!(recovered.job("MASK").unwrap().provenance()), expected);
    assert!(
        JobProvenance::from_result(&json!({
            "sha256":"bad", "samVersion":"bad version", "maskHeight":1.5,
            "maskWidth":9_007_199_254_740_992_u64, "internalField":"not public"
        }))
        .is_none()
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn pixal3d_live_and_recovered_jobs_use_glb_media_urls_and_cannot_enhance() {
    let seen = Arc::new(RwLock::new(Vec::<Value>::new()));
    let capture = seen.clone();
    let raw = json!({
        "id":"RECOVERED", "status":"completed", "model":{"id":"pixal3d_int8_i23d","type":"image"},
        "imageCount":1, "completedWorkerJobs":[{"id":"MODEL","status":"jobCompleted", "result":{"sha256":"A".repeat(64)}}]
    });
    let app = Router::new()
        .route(
            "/v1/media/downloadUrl",
            get(move |Query(query): Query<BTreeMap<String, String>>| {
                capture.write().push(json!(query));
                async { Json(json!({"data":{"downloadUrl":"https://example.test/object.glb"}})) }
            }),
        )
        .route(
            "/api/v1/artist/projects/sync",
            get(move || {
                let raw = raw.clone();
                async move { Json(json!({"activeProjects":[],"unclaimedCompletedProjects":[raw]})) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = SogniClient::builder()
        .app_id("artifact-fixture")
        .api_key("local-fixture")
        .rest_endpoint(Url::parse(&format!("http://{address}/")).unwrap())
        .socket_endpoint(Url::parse(&format!("ws://{address}/")).unwrap())
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(1))
        .build()
        .await
        .unwrap();
    let project = Project::new(
        "LIVE".into(),
        json!({"type":"image","modelId":"pixal3d_int8_i23d"}),
        false,
        Arc::downgrade(&client.projects.inner),
    );
    client
        .projects
        .inner
        .projects
        .write()
        .insert(project.id(), project.clone());
    handle_job_result(
        &client.projects.inner,
        &json!({"jobID":"LIVE","imgID":"MODEL"}),
    )
    .await;
    let job = project.job("MODEL").unwrap();
    assert_eq!(job.media_type(), "model");
    assert_eq!(
        job.get_result_url().await.unwrap(),
        "https://example.test/object.glb"
    );
    assert!(
        job.enhance("light", None)
            .await
            .unwrap_err()
            .to_string()
            .contains("only available for images")
    );
    let recovered = client.projects.recover_project("RECOVERED").await.unwrap();
    let recovered_job = recovered.job("MODEL").unwrap();
    assert_eq!(recovered_job.media_type(), "model");
    assert_eq!(
        recovered_job.get_result_url().await.unwrap(),
        "https://example.test/object.glb"
    );
    assert_eq!(
        recovered_job.provenance().unwrap().sha256,
        Some("a".repeat(64))
    );
    assert_eq!(
        *seen.read(),
        vec![
            json!({"jobId":"LIVE","id":"MODEL","type":"complete","contentType":"model/gltf-binary"}),
            json!({"jobId":"RECOVERED","id":"MODEL","type":"complete","contentType":"model/gltf-binary"}),
        ]
    );
    client.close().await.unwrap();
    server.abort();
}
