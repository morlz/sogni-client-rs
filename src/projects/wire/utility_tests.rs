use super::*;

fn options(media: &str) -> ModelOptions {
    ModelOptions {
        model_id: "fixture".into(),
        media_type: media.into(),
        raw: json!({"sampler":{"allowed":[],"default":null},"scheduler":{"allowed":[],"default":null}}),
    }
}

fn sam3_request(prompt: Value) -> ProjectRequest {
    ProjectRequest::image(SAM3_MODEL_ID, "")
        .param("startingImage", true)
        .param("sam3Prompt", prompt)
}

#[test]
fn sam3_defaults_and_single_mask_wire_override_conflicting_options() {
    let request = ProjectRequest::image(SAM3_MODEL_ID, "")
        .number_of_media(4)
        .param("numberOfPreviews", 5)
        .param("outputFormat", "jpg")
        .param("startingImage", true)
        .sam3_prompt(Sam3ImagePrompt {
            points: vec![Sam3PromptPoint {
                x: 0.42,
                y: 0.61,
                label: Sam3PointLabel::Positive,
            }],
            ..Default::default()
        });
    let wire = build_job_request("MASK", &request.params, &options("image"), None).unwrap();
    assert_eq!(wire["numberOfImages"], 1);
    assert_eq!(wire["previews"], 0);
    assert_eq!(wire["outputFormat"], "png");
    assert_eq!(
        wire["keyFrames"][0]["sam3Prompt"],
        json!({
            "points":[{"x":0.42,"y":0.61,"label":"positive"}],
            "boxes":[], "threshold":0.5, "multimask":true,"applyMask":false,
        })
    );
    let request = sam3_request(json!({"text":"  teapot  ","threshold":0,"multimask":false}));
    let wire = build_job_request("TEXT", &request.params, &options("image"), None).unwrap();
    assert_eq!(
        wire["keyFrames"][0]["sam3Prompt"],
        json!({
            "points":[], "boxes":[], "text":"teapot", "threshold":0.0, "applyMask":false,
        })
    );
}

#[test]
fn sam3_rejects_invalid_geometry_combinations_and_unknown_fields() {
    let point = json!({"x":0.5,"y":0.5,"label":"positive"});
    let selection = json!({"x0":0,"y0":0,"x1":1,"y1":1});
    for (prompt, message) in [
        (json!([]), "sam3Prompt must be an object"),
        (
            json!({}),
            "requires at least one point, box, or text prompt",
        ),
        (
            json!({"text":"test","extra":1}),
            "contains unsupported fields: extra",
        ),
        (
            json!({"points":{}}),
            "points must contain at most 32 entries",
        ),
        (json!({"points":[null]}), "points[0] must be an object"),
        (
            json!({"points":[{"x":"0.5","y":0.5,"label":"positive"}]}),
            "points[0].x must be a finite normalized coordinate",
        ),
        (
            json!({"points":[{"x":1.1,"y":0.5,"label":"positive"}]}),
            "points[0].x must be a finite normalized coordinate",
        ),
        (
            json!({"points":[{"x":0.5,"y":0.5,"label":"other"}]}),
            "label must be \"positive\" or \"negative\"",
        ),
        (
            json!({"points":[{"x":0.5,"y":0.5,"label":"positive","id":1}]}),
            "points[0] contains unsupported fields",
        ),
        (
            json!({"points":vec![point.clone();33]}),
            "points must contain at most 32 entries",
        ),
        (
            json!({"boxes":vec![selection.clone();17]}),
            "boxes must contain at most 16 entries",
        ),
        (
            json!({"boxes":[{"x0":1,"y0":0,"x1":0,"y1":1}]}),
            "must have x0 < x1 and y0 < y1",
        ),
        (json!({"text":" "}), "text must contain 1 to 240 characters"),
        (
            json!({"text":"😀".repeat(121)}),
            "text must contain 1 to 240 characters",
        ),
        (json!({"text":null}), "text must be a string"),
        (
            json!({"text":"test","points":[point.clone()]}),
            "cannot combine text and point prompts",
        ),
        (
            json!({"points":[point],"boxes":[selection.clone(),selection]}),
            "at most one box when point prompts are present",
        ),
        (
            json!({"text":"test","threshold":null}),
            "threshold must be a finite number from 0 to 1",
        ),
        (
            json!({"text":"test","threshold":1.1}),
            "threshold must be a finite number from 0 to 1",
        ),
        (
            json!({"text":"test","multimask":1}),
            "multimask must be a boolean",
        ),
    ] {
        let request = sam3_request(prompt);
        let error =
            build_job_request("MASK", &request.params, &options("image"), None).unwrap_err();
        assert!(
            error.to_string().contains(message),
            "{error}; expected {message}"
        );
    }
}

#[test]
fn image_utilities_require_sources_and_the_canonical_sam3_model() {
    for (request, message) in [
        (
            ProjectRequest::image(SAM3_MODEL_ID, ""),
            "SAM3 image segmentation requires startingImage",
        ),
        (
            ProjectRequest::image(SAM3_MODEL_ID, "").param("startingImage", true),
            "SAM3 image segmentation requires sam3Prompt",
        ),
        (
            sam3_request(json!({"text":"cat"})).param("modelId", "sam3-image-segment"),
            "sam3Prompt is only supported by sam3_image_segment_bf16",
        ),
        (
            ProjectRequest::image(PIXAL3D_MODEL_ID, ""),
            "Pixal3D reconstruction requires startingImage",
        ),
    ] {
        let error =
            build_job_request("UTILITY", &request.params, &options("image"), None).unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
    let request = ProjectRequest::image(PIXAL3D_MODEL_ID, "teapot")
        .param("startingImage", true)
        .param("outputFormat", "jpg");
    let wire = build_job_request("GLB", &request.params, &options("image"), None).unwrap();
    assert_eq!(wire["outputFormat"], "glb");
    assert_eq!(wire["keyFrames"][0]["hasStartingImage"], true);
}

#[test]
fn world_receipts_validate_stages_hashes_and_keyframe_placement() {
    let request = ProjectRequest::image("krea2_identity_edit_sogni_v0_3_alpha", "edit")
        .param("appSource", "sogni-world")
        .world_generation_receipt(WorldGenerationReceiptRequest::TargetStill {
            source_image_sha256: "A".repeat(64),
            selection_hash: "B".repeat(64),
        });
    let wire = build_job_request("STILL", &request.params, &options("image"), None).unwrap();
    assert!(wire.get("worldGenerationReceipt").is_none());
    assert_eq!(
        wire["keyFrames"][0]["worldGenerationReceipt"],
        json!({
            "stage":"target_still", "sourceImageSha256":"a".repeat(64), "selectionHash":"b".repeat(64),
        })
    );
    let transition = ProjectRequest::video("minimax-h3-fastvideo-int8_flf2v_turbo", "move")
        .param("appSource", "sogni-world")
        .param("referenceImage", true)
        .param("referenceImageEnd", true)
        .world_generation_receipt(WorldGenerationReceiptRequest::Transition {
            first_frame_sha256: "C".repeat(64),
            last_frame_sha256: "D".repeat(64),
        });
    let wire =
        build_job_request("TRANSITION", &transition.params, &options("video"), None).unwrap();
    assert_eq!(
        wire["keyFrames"][0]["worldGenerationReceipt"]["firstFrameSha256"],
        "c".repeat(64)
    );
    for (invalid, message) in [
        (
            request.clone().param("worldGenerationReceipt", json!({})),
            "stage must be target_still or transition",
        ),
        (
            request.clone().param(
                "worldGenerationReceipt",
                json!({"stage":"target_still","sourceImageSha256":"z".repeat(64)}),
            ),
            "sourceImageSha256 must be a SHA-256 hex digest",
        ),
    ] {
        let error =
            build_job_request("RECEIPT", &invalid.params, &options("image"), None).unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
}

#[test]
fn wan3_rejects_retired_option_even_when_false_or_null() {
    for value in [json!(true), json!(false), Value::Null] {
        let request = ProjectRequest::video("wan3.0-video", "a kite").param("smartDuration", value);
        let error =
            build_job_request("WAN3", &request.params, &options("video"), None).unwrap_err();
        assert!(error.to_string().contains("smartDuration has been retired"));
    }
    let request = ProjectRequest::video("wan3.0-video", "a kite").duration(30.0);
    let wire = build_job_request("WAN3", &request.params, &options("video"), None).unwrap();
    assert_eq!(wire["keyFrames"][0]["frames"], 901);
    assert!(wire["keyFrames"][0].get("smartDuration").is_none());
}
