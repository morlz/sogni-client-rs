use super::*;
use crate::{
    RELAYVSR_X2_VIDEO_UPSCALE_MODEL_ID, RELAYVSR_X4_VIDEO_UPSCALE_MODEL_ID,
    SEEDREAM_5_5_UNCENSORED_MODEL_ID,
};

fn frame(model: &str, media_type: &str, changes: Value) -> Result<Value> {
    let mut params = json!({
        "modelId":model,"type":media_type,"positivePrompt":"","numberOfMedia":1,
    })
    .as_object()
    .unwrap()
    .clone();
    params.extend(changes.as_object().unwrap().clone());
    let options = ModelOptions {
        model_id: model.into(),
        media_type: media_type.into(),
        raw: json!({
            "steps":{"min":1,"max":50,"default":8}, "guidance":{"min":1,"max":1,"default":1},
            "sampler":{"allowed":["euler"],"default":"euler"}, "scheduler":{"allowed":[],"default":null}
        }),
    };
    Ok(build_job_request("SDK584", &params, &options, None)?["keyFrames"][0].clone())
}

#[test]
fn relayvsr_leaves_unreported_source_timing_unset_while_flashvsr_stays_unchanged() {
    let params = json!({"referenceVideo":true,"upscaleResolution":2160});
    for model in [
        RELAYVSR_X2_VIDEO_UPSCALE_MODEL_ID,
        RELAYVSR_X4_VIDEO_UPSCALE_MODEL_ID,
    ] {
        let key = frame(model, "video", params.clone()).unwrap();
        assert_eq!(key["upscaleResolution"], 2160.0);
        assert_eq!(key["seed"], 0);
        assert_eq!(key["steps"], 1);
        assert_eq!(key["generateAudio"], true);
        for field in [
            "frames",
            "fps",
            "width",
            "height",
            "detailPreference",
            "processingSpeed",
        ] {
            assert!(key.get(field).is_none(), "{field}: {key}");
        }
        let mut long = params.clone();
        long["width"] = json!(2160);
        long["height"] = json!(4838);
        assert!(
            frame(model, "video", long)
                .unwrap_err()
                .to_string()
                .contains("3840")
        );
        let mut seed = params.clone();
        seed["seed"] = json!(0);
        assert!(
            frame(model, "video", seed)
                .unwrap_err()
                .to_string()
                .contains("RelayVSR has no seed")
        );
    }
    let flash = "flashvsr_v1.1_tiny_long_bf16";
    assert!(
        frame(flash, "video", params)
            .unwrap_err()
            .to_string()
            .contains("Choose 1080p or 1440p")
    );
    let key = frame(
        flash,
        "video",
        json!({"referenceVideo":true,"upscaleResolution":1440,"seed":42}),
    )
    .unwrap();
    assert_eq!(key["detailPreference"], "stable");
    assert_eq!(key["processingSpeed"], "stable");
    assert_eq!(key["seed"], 42);
}

#[test]
fn seedream_55_keeps_layer_controls_and_rejects_invalid_modes() {
    let model = SEEDREAM_5_5_UNCENSORED_MODEL_ID;
    let params = json!({
        "contextImages":[true],"seedreamLayerDecomposition":true,"seedreamLayerSize":"1.5K",
        "seedreamLayerMaxImages":100,"seedreamLayerTextRecognition":false,
    });
    let key = frame(model, "image", params.clone()).unwrap();
    for field in [
        "seedreamLayerDecomposition",
        "seedreamLayerSize",
        "seedreamLayerMaxImages",
        "seedreamLayerTextRecognition",
    ] {
        assert_eq!(key[field], params[field], "{field}");
    }
    let mut invalid = params;
    invalid["contextImages"] = json!([true, true]);
    assert!(
        frame(model, "image", invalid)
            .unwrap_err()
            .to_string()
            .contains("exactly one")
    );
    let transparent = json!({"seedreamBackground":"transparent","outputFormat":"jpg"});
    assert!(
        frame(model, "image", transparent.clone())
            .unwrap_err()
            .to_string()
            .contains("PNG or WebP")
    );
    assert!(
        frame("seedream-5-0-pro-uncensored", "image", transparent)
            .unwrap_err()
            .to_string()
            .contains("requires seedream-5-5")
    );
    let key = frame(
        model,
        "image",
        json!({"sizePreset":"custom","width":2180,"height":2090,
        "seedreamBackground":"auto","outputFormat":"png"}),
    )
    .unwrap();
    assert_eq!(key["width"], 2180.0);
    assert_eq!(key["seedreamBackground"], "auto");
}

#[test]
fn sam3_opt_in_preserves_drawn_anchor_objects_and_original_source_geometry() {
    let model = "sam3_video_segment_fp16";
    let prompt = json!({
        "frames":[{"frameIndex":0,"text":"lamp"},{"frameIndex":8,"mask":{
            "polygons":[[[0.1,0.1],[0.2,0.1],[0.2,0.2]]]
        }}],"memory":"anchor","combine":"union"
    });
    let key = frame(
        model,
        "video",
        json!({"referenceVideo":true,"sam3VideoPrompt":prompt,
        "width":1920,"height":1080,"frames":255,"fps":30000.0/1001.0}),
    )
    .unwrap();
    assert_eq!(key["sam3VideoPrompt"], prompt);
    assert_eq!(key["frames"], 255);
    assert_eq!(key["fps"], json!(30000.0 / 1001.0));
    let mut points = json!({"points":[{"x":0.25,"y":0.75,"label":"positive"}],"objectIndex":1});
    assert!(
        frame(
            model,
            "video",
            json!({"referenceVideo":true,"sam3VideoPrompt":points})
        )
        .unwrap_err()
        .to_string()
        .contains("objectIndex 0")
    );
    points["objectIndex"] = json!(0);
    let key = frame(model,"video",json!({"referenceVideo":true,"sam3VideoPrompt":points,"frames":255,"width":1920,"height":1080,"fps":24})).unwrap();
    assert_eq!(key["sam3VideoPrompt"]["objectIndex"], 0);
    assert_eq!(key["frames"], 255);
    assert_eq!(key["width"], 1920);
}

#[test]
fn minimax_h3_prism_uses_eight_steps_and_the_existing_reference_slots() {
    let model = "minimax-h3-prism-int8_r2v";
    let params = json!({"referenceImage":true,"steps":8,"frames":141,"keyframes":[{"image":true,"frameIndex":72}]});
    let key = frame(model, "video", params.clone()).unwrap();
    assert_eq!(key["steps"], 8);
    assert_eq!(key["frames"], 141);
    assert_eq!(key["hasKeyframeImage1"], true);
    assert_eq!(key["keyframeFrameIndices"], json!([72]));
    let mut invalid = params;
    invalid["steps"] = json!(4);
    assert!(
        frame(model, "video", invalid)
            .unwrap_err()
            .to_string()
            .contains("Prism steps are fixed at 8")
    );
}
