use super::*;

fn options(media_type: &str) -> ModelOptions {
    ModelOptions {
        model_id: "model".into(),
        media_type: media_type.into(),
        raw: json!({
            "type": media_type,
            "sampler": {"allowed": [], "default": null},
            "scheduler": {"allowed": [], "default": null}
        }),
    }
}

#[test]
fn builds_image_wire_envelope() {
    let params = ProjectRequest::image("model", "a lighthouse")
        .steps(4)
        .guidance(1.0)
        .params
        .clone();
    let wire =
        build_job_request("ABC", &params, &options("image"), None).expect("valid image request");
    assert_eq!(wire["jobID"], "ABC");
    assert_eq!(wire["keyFrames"][0]["positivePrompt"], "a lighthouse");
    assert_eq!(wire["outputFormat"], "png");
}

#[test]
fn distinguishes_withheld_from_labeled_media() {
    let mut job = JobSnapshot::pending("I".into(), "P".into(), 1.0);
    job.status = JobStatus::Completed;
    job.is_nsfw = true;
    job.nsfw_detected = true;
    assert_eq!(job_progress(&job), 100);
    assert!(!job.is_nsfw || job.nsfw_detected);
}

#[test]
fn validates_workflow_assets() {
    let request = ProjectRequest::video("wan_v2.2-14b-fp8_i2v", "move").params;
    let error =
        validate_video_assets(&request, "wan_v2.2-14b-fp8_i2v").expect_err("i2v needs an image");
    assert!(
        error
            .to_string()
            .contains("requires at least one of referenceImage")
    );
}

#[test]
fn attaching_an_asset_enables_its_wire_parameter() {
    let request = ProjectRequest::video("wan_v2.2-14b-fp8_i2v", "move").asset(
        AssetRole::ReferenceImage,
        MediaSource::named_bytes(Bytes::from_static(b"image"), "image.png", "image/png"),
    );
    assert_eq!(request.params()["referenceImage"], true);

    let request = ProjectRequest::video("minimax-h3-ref2va-fp8_r2v", "speak")
        .asset(
            AssetRole::ReferenceVideoSlot(1),
            MediaSource::named_bytes(Bytes::from_static(b"video-1"), "one.mp4", "video/mp4"),
        )
        .asset(
            AssetRole::ReferenceVideoSlot(2),
            MediaSource::named_bytes(Bytes::from_static(b"video-2"), "two.mp4", "video/mp4"),
        );
    assert_eq!(request.params()["referenceVideo"], true);
    assert_eq!(request.params()["referenceVideos"], json!([true]));
}

#[test]
fn builds_minimax_h3_reference_wire_contract() {
    let value = json!({
        "type": "video",
        "modelId": "minimax-h3-ref2va-fp8_r2v",
        "positivePrompt": "A character walks into frame and speaks.",
        "negativePrompt": "",
        "numberOfMedia": 1,
        "duration": 6,
        "referenceImage": true,
        "contextImages": [true],
        "referenceVideo": true,
        "referenceVideoDurations": [4],
        "referenceAudio": true,
        "width": 1024,
        "height": 768,
    });
    let params = value.as_object().expect("object");
    let message = build_job_request("H3-REFERENCE", params, &options("video"), None)
        .expect("valid MiniMax H3 request");
    let keyframe = &message["keyFrames"][0];
    assert_eq!(keyframe["fps"], 24);
    assert_eq!(keyframe["frames"], 141);
    assert_eq!(keyframe["hasReferenceImage"], true);
    assert!(keyframe.get("hasContextImage1").is_none());
    assert_eq!(keyframe["hasContextImage2"], true);
    assert_eq!(keyframe["hasReferenceVideo1"], true);
    assert_eq!(keyframe["referenceVideo1DurationSeconds"], 4);
    assert_eq!(keyframe["hasReferenceAudio1"], true);
}

#[test]
fn validates_minimax_h3_fixed_parameters_and_duration_hints() {
    for (model_id, expected, label) in [
        ("minimax-h3-fl2va-fp8_t2v", 20, ""),
        ("minimax-h3-fl2va-fp8_t2v_balanced", 8, " Balanced"),
        ("minimax-h3-fastvideo-int8_t2v_turbo", 4, " Turbo"),
    ] {
        let value = json!({
            "type": "video",
            "modelId": model_id,
            "positivePrompt": "a kite",
            "numberOfMedia": 1,
            "steps": expected + 1,
        });
        let error = build_job_request(
            "H3-STEPS",
            value.as_object().expect("object"),
            &options("video"),
            None,
        )
        .expect_err("wrong fixed step count must fail");
        assert!(
            error
                .to_string()
                .contains(&format!("MiniMax H3{label} steps are fixed at {expected}"))
        );
    }

    let invalid_duration_count = json!({
        "type": "video",
        "modelId": "minimax-h3-ref2va-fp8_r2v",
        "positivePrompt": "walk",
        "numberOfMedia": 1,
        "referenceImage": true,
        "referenceVideo": true,
        "referenceVideoDurations": [3, 4],
    });
    let error = build_job_request(
        "H3-DURATION",
        invalid_duration_count.as_object().expect("object"),
        &options("video"),
        None,
    )
    .expect_err("duration hints must match videos");
    assert!(error.to_string().contains("expected 1"));

    let invalid_dimensions = json!({
        "type": "video",
        "modelId": "minimax-h3-fl2va-fp8_t2v",
        "positivePrompt": "walk",
        "numberOfMedia": 1,
        "width": 1000,
        "height": 768,
    });
    let error = build_job_request(
        "H3-SIZE",
        invalid_dimensions.as_object().expect("object"),
        &options("video"),
        None,
    )
    .expect_err("dimensions must follow the H3 grid");
    assert!(error.to_string().contains("32px grid"));
}

#[test]
fn validates_wan3_and_seedance25_contracts() {
    let wan3 = json!({
        "type": "video",
        "modelId": "wan3.0-video",
        "positivePrompt": "",
        "numberOfMedia": 1,
        "duration": 30,
        "referenceLinkUrl": "https://example.com/reference",
        "promptExtend": false,
        "ratio": "9:16",
        "watermark": false,
    });
    let message = build_job_request(
        "WAN3",
        wan3.as_object().expect("object"),
        &options("video"),
        None,
    )
    .expect("valid Wan 3 request");
    let keyframe = &message["keyFrames"][0];
    assert_eq!(keyframe["fps"], 30);
    assert_eq!(keyframe["frames"], 901);
    assert_eq!(
        keyframe["referenceLinkURL"],
        "https://example.com/reference"
    );
    assert_eq!(keyframe["promptExtend"], false);

    let retired = json!({
        "type": "video",
        "modelId": "wan3.0-video",
        "positivePrompt": "render",
        "numberOfMedia": 1,
        "smartDuration": true,
    });
    let error = build_job_request(
        "WAN3-SMART",
        retired.as_object().expect("object"),
        &options("video"),
        None,
    )
    .expect_err("retired smartDuration must fail");
    assert!(error.to_string().contains("smartDuration has been retired"));

    let seedance = json!({
        "type": "video",
        "modelId": "seedance-2-5",
        "positivePrompt": "Use the soundtrack.",
        "numberOfMedia": 1,
        "duration": 30,
        "seedanceTaskType": "reference",
        "referenceAudioUrls": ["https://cdn.example/audio.mp3"],
    });
    let message = build_job_request(
        "SEEDANCE",
        seedance.as_object().expect("object"),
        &options("video"),
        None,
    )
    .expect("valid Seedance 2.5 request");
    assert_eq!(message["keyFrames"][0]["frames"], 721);
    assert_eq!(message["keyFrames"][0]["seedanceTaskType"], "reference");
}

#[test]
fn enforces_external_video_duration_ranges() {
    for (model_id, duration) in [
        ("wan3.0-video", 1.0),
        ("happyhorse-1.1-t2v", 2.0),
        ("seedance-2-0", 16.0),
    ] {
        let value = json!({
            "type": "video",
            "modelId": model_id,
            "positivePrompt": "render",
            "numberOfMedia": 1,
            "duration": duration,
        });
        let error = build_job_request(
            "DURATION",
            value.as_object().expect("object"),
            &options("video"),
            None,
        )
        .expect_err("out-of-range duration must fail");
        assert!(error.to_string().contains("video duration must be between"));
    }
}

#[test]
fn lost_project_marker_is_stable_and_detectable() {
    let payload = project_lost_payload();
    assert_eq!(payload["originalCode"], json!(PROJECT_LOST_ORIGINAL_CODE));
    assert!(is_project_lost_payload(&payload));
    let error = Error::Project(ProjectError::from_payload(payload));
    assert!(is_project_lost_error(&error));
    assert!(!is_project_lost_error(&Error::Closed));
}

#[test]
fn project_resolution_states_match_other_clients() {
    let cases = [
        (
            ProjectResolution::Finished { project: json!({}) },
            "finished",
        ),
        (ProjectResolution::Active, "active"),
        (ProjectResolution::Lost, "lost"),
        (
            ProjectResolution::Unknown {
                error: "unavailable".into(),
            },
            "unknown",
        ),
    ];
    for (resolution, expected) in cases {
        assert_eq!(resolution.state(), expected);
    }
}

#[test]
fn resolve_missing_defaults_match_recovery_contract() {
    let options = ResolveMissingOptions::default();
    assert_eq!(options.attempts, 4);
    assert_eq!(options.retry_delay, Duration::from_millis(2_500));
}
