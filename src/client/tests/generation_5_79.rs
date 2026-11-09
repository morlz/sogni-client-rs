use super::{
    project_wire_server::{Fixture, KEY},
    *,
};
use crate::{
    AssetRole, MediaSource, ProjectRequest, SAM3_VIDEO_SEGMENT_MODEL_ID, Sam3VideoPrompt,
    calculate_video_frames, get_video_workflow_type, is_video_segmentation_model,
};
use serde_json::Value;

async fn client(fixture: &Fixture) -> SogniClient {
    SogniClient::builder()
        .app_id("generation-579-fixture")
        .api_key(KEY)
        .rest_endpoint(format!("http://{}/", fixture.address).parse().unwrap())
        .socket_endpoint(format!("ws://{}/", fixture.address).parse().unwrap())
        .request_timeout(Duration::from_secs(3))
        .connect_timeout(Duration::from_secs(3))
        .build()
        .await
        .unwrap()
}

async fn wire(fixture: &mut Fixture) -> Value {
    tokio::time::timeout(Duration::from_secs(3), fixture.wire.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn sam3_video_uses_full_source_upload_and_preserves_receipt_on_completion() {
    let mut fixture = Fixture::start().await;
    let client = client(&fixture).await;
    let model = SAM3_VIDEO_SEGMENT_MODEL_ID;
    let fps = 30000.0 / 1001.0;
    assert!(is_video_segmentation_model(model));
    assert_eq!(get_video_workflow_type(model), Some("v2v"));
    assert_eq!(
        calculate_video_frames(model, 300.0 / fps, fps, None, None).unwrap(),
        300
    );
    let options = client
        .projects
        .get_model_options(model, false)
        .await
        .unwrap();
    assert_eq!(options.raw["task"], "video-segmentation");
    assert_eq!(options.raw["preservesSourceTiming"], true);
    let request = ProjectRequest::video(model, "")
        .sam3_video_prompt(Sam3VideoPrompt {
            text: Some("unicorn horn".into()),
            frame_index: Some(126),
            ..Default::default()
        })
        .asset(
            AssetRole::ReferenceVideo,
            MediaSource::named_bytes("synthetic full source", "input.mp4", "video/mp4"),
        );
    assert!(
        client
            .projects
            .create(request.clone().param("frames", 126))
            .await
            .is_err()
    );
    assert!(fixture.wire.try_recv().is_err());
    assert!(
        !fixture
            .http
            .lock()
            .iter()
            .any(|request| request.path.contains("uploadUrl"))
    );
    let project = client.projects.create(request).await.unwrap();
    let request = wire(&mut fixture).await;
    let frame = &request["keyFrames"][0];
    assert_eq!(frame["referenceVideoContentType"], "video/mp4");
    assert_eq!(frame["sam3VideoPrompt"]["frameIndex"], 126);
    assert!(frame.get("frames").is_none());
    assert!(frame.get("fps").is_none());
    assert_eq!(request["outputFormat"], "mp4");
    let urls = project
        .wait_for_completion(Some(Duration::from_secs(3)))
        .await
        .unwrap();
    assert_eq!(urls, ["https://example.test/result.mp4"]);
    let receipt = project.job("JOB-1").unwrap().provenance().unwrap();
    assert_eq!(receipt.source_video_sha256, Some("a".repeat(64)));
    assert_eq!(receipt.mask_frames, Some(255));
    assert_eq!(receipt.mask_fps, Some(fps));
    assert_eq!(receipt.sam_version.as_deref(), Some("sam3.1@fixture"));
    let http = fixture.http.lock().clone();
    let registration = http
        .iter()
        .find(|request| request.path == "/v1/media/uploadUrl")
        .unwrap();
    assert_eq!(registration.query["type"], "referenceVideo");
    assert_eq!(registration.query["jobId"], project.id());
    assert_eq!(
        http.iter()
            .find(|request| request.method == "PUT")
            .unwrap()
            .body,
        b"synthetic full source"
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn seedance_draft_and_final_share_result_metadata_with_durable_lookup() {
    let mut fixture = Fixture::start().await;
    let client = client(&fixture).await;
    let draft = client
        .projects
        .create(
            ProjectRequest::video("seedance-2-5", "A wave rolls in")
                .dimensions(864, 496)
                .duration(8.0)
                .seedance_draft(true),
        )
        .await
        .unwrap();
    assert_eq!(
        wire(&mut fixture).await["keyFrames"][0]["seedanceDraft"],
        true
    );
    draft
        .wait_for_completion(Some(Duration::from_secs(3)))
        .await
        .unwrap();
    let job = draft.jobs().pop().unwrap();
    let receipt = job.seedance_draft_receipt();
    assert_eq!(receipt.seedance_draft, Some(true));
    assert_eq!(
        receipt.seedance_draft_expires_at.as_deref(),
        Some("2026-11-17T20:00:00.000Z")
    );

    let final_project = client
        .projects
        .create(
            ProjectRequest::video("seedance-2-5", "")
                .seedance_draft_job_id(job.id())
                .return_last_frame(true),
        )
        .await
        .unwrap();
    let final_wire = wire(&mut fixture).await;
    assert_eq!(final_wire["keyFrames"][0]["seedanceDraftJobId"], job.id());
    assert!(final_wire["keyFrames"][0].get("frames").is_none());
    final_project
        .wait_for_completion(Some(Duration::from_secs(3)))
        .await
        .unwrap();
    let final_job = final_project.jobs().pop().unwrap();
    assert_eq!(
        final_job.seedance_draft_receipt().seedance_draft_job_id,
        Some(job.id())
    );
    assert_eq!(
        final_job.snapshot().last_frame_key.as_deref(),
        Some("final-frame.png")
    );
    assert!(
        final_job
            .get_last_frame_url()
            .await
            .unwrap()
            .ends_with("/fixture-result")
    );
    assert!(fixture.http.lock().iter().any(|request| {
        request.path == "/v1/media/downloadUrl" && request.query["artifact"] == "lastFrame"
    }));
    *fixture.status.lock() = Some(json!({
        "id": final_project.id(), "status":"completed","finished":true,
        "model":{"id":"seedance-2-5","type":"video"}, "completedWorkerJobs":[{
            "imgID": final_job.id(), "status":"jobCompleted", "resultUrl":"https://example.test/result.mp4",
            "result":{"seedanceDraftJobId":job.id(),"privateReceiptField":"omit"},
        }],
    }));
    let result = client
        .projects
        .get_result(&final_project.id(), None)
        .await
        .unwrap();
    assert_eq!(
        result.jobs[0].seedance.seedance_draft_job_id,
        Some(job.id())
    );
    let public = json!(&result.jobs[0]);
    assert!(public.get("seedanceDraft").is_none());
    assert!(public.get("privateReceiptField").is_none());
    client.close().await.unwrap();
}

#[tokio::test]
async fn generation_quotes_forward_priority_monthly_gif_and_draft_options() {
    let fixture = Fixture::start().await;
    let client = client(&fixture).await;
    let mut image = json!({
        "model":"gpt-image-2.6-flare","imageCount":1,"stepCount":1,"previewCount":0,
        "outputFormat":"gif","width":320,"sizePreset":"portrait","contextImages":2,
        "gptImageFrameCount":9,"gptImageFrameDurationMs":200,"gptImageComplexity":"low",
        "priorityMode":true,"includeMonthlyFairUse":false,
    });
    let ordinary = client.projects.estimate_cost(&image).await.unwrap();
    assert_eq!(ordinary.daily_fair_use_pct, Some(3.0));
    assert_eq!(ordinary.monthly_fair_use_pct, None);
    image["includeMonthlyFairUse"] = json!(true);
    let expanded = client.projects.estimate_cost(&image).await.unwrap();
    assert_eq!(expanded.monthly_fair_use_pct, Some(0.15));
    let mut video = json!({
        "model":"seedance-2-5","tokenType":"spark","width":864,"height":496,"duration":8,"fps":24,
        "seedanceDraft":true, "priorityMode":true,"includeMonthlyFairUse":true,
    });
    assert_eq!(
        client
            .projects
            .estimate_video_cost(&video)
            .await
            .unwrap()
            .monthly_fair_use_pct,
        Some(0.15)
    );
    video["seedanceDraft"] = json!(false);
    video["seedanceDraftJobId"] = json!("draft&id");
    client.projects.estimate_video_cost(&video).await.unwrap();
    video["seedanceDraft"] = json!(true);
    assert!(client.projects.estimate_video_cost(&video).await.is_err());
    assert_eq!(client.projects.estimate_audio_cost(&json!({
        "model":"minimax_music3","tokenType":"spark","duration":30,"steps":1,"numberOfMedia":1,
        "priorityMode":true,"includeMonthlyFairUse":true,
    })).await.unwrap().monthly_fair_use_pct, Some(0.15));
    let http = fixture.http.lock().clone();
    let images: Vec<_> = http
        .iter()
        .filter(|request| request.path.contains("/job/estimate/"))
        .collect();
    assert_eq!(images.len(), 2);
    assert!(images[0].path.ends_with("/0/0/320/320/0/_/2"));
    assert!(!images[0].query.contains_key("includeMonthlyFairUse"));
    assert_eq!(images[1].query["gptImageFrameCount"], "9");
    assert_eq!(images[1].query["gptImageFrameDurationMs"], "200");
    assert_eq!(images[1].query["gptImageComplexity"], "low");
    let videos: Vec<_> = http
        .iter()
        .filter(|request| request.path.contains("/job-video/estimate/"))
        .collect();
    assert_eq!(videos.len(), 2);
    assert_eq!(videos[0].path, videos[1].path);
    assert_eq!(videos[0].query["seedanceDraft"], "1");
    assert!(!videos[1].query.contains_key("seedanceDraft"));
    assert_eq!(videos[1].query["seedanceDraftJobId"], "draft&id");
    client.close().await.unwrap();
}
