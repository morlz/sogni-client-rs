use serde_json::json;
use sogni_client::{WorkflowBillingOptions, WorkflowStart};

use crate::cli::Args;

pub(super) fn start(args: &Args) -> WorkflowStart {
    let prompt = args.prompt.join(" ");
    WorkflowStart {
        input: Some(json!({
            "title": "Generated keyframe to video",
            "steps": [
                {
                    "id": "keyframe",
                    "toolName": "generate_image",
                    "arguments": {
                        "prompt": prompt,
                        "negativePrompt": args.negative_prompt,
                        "width": args.width,
                        "height": args.height,
                        "model": args.image_model,
                        "numberOfVariations": args.number_of_media,
                        "seed": args.seed,
                    }
                },
                {
                    "id": "clip",
                    "toolName": "generate_video",
                    "arguments": {
                        "prompt": args.video_prompt.as_deref().unwrap_or(&prompt),
                        "negativePrompt": args.negative_prompt,
                        "width": args.width,
                        "height": args.height,
                        "duration": args.duration,
                        "videoModel": args.video_model,
                        "numberOfVariations": args.number_of_media,
                    },
                    // The durable executor resolves the keyframe by media index; no
                    // temporary result URL has to be copied into the second step.
                    "dependsOn": [{
                        "sourceStepId": "keyframe",
                        "sourceArtifactIndex": 0,
                        "targetArgument": "referenceImageIndices",
                        "mediaType": "image",
                        "transform": "image_index",
                        "required": true,
                    }]
                }
            ]
        })),
        token_type: Some(args.token_type.clone()),
        billing_mode: Some(args.billing_mode.clone()),
        app_source: Some("sogni-client-rs-example".into()),
        ..WorkflowStart::default()
    }
}

pub(super) fn billing(args: &Args) -> WorkflowBillingOptions {
    WorkflowBillingOptions {
        token_type: Some(args.token_type.clone()),
        billing_mode: Some(args.billing_mode.clone()),
        app_source: Some("sogni-client-rs-example".into()),
        ..WorkflowBillingOptions::default()
    }
}
