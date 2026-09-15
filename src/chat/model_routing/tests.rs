use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};

fn fixtures() -> Value {
    serde_json::from_str(include_str!("../fixtures/upstream-routing.json")).unwrap()
}

#[test]
fn selectors_match_fresh_typescript_runtime() {
    for case in fixtures()["selectors"].as_array().unwrap() {
        assert_eq!(
            json!(resolve_hosted_tool_model_selector(
                case["tool"].as_str().unwrap(),
                &case["args"]
            )),
            case["expected"],
            "{case}"
        );
    }
    assert_eq!(
        resolve_hosted_tool_model_selector("generate_image", &json!({"model":"_gpt_image_"}))
            .as_deref(),
        Some("_gpt_image_")
    );
}

#[test]
fn defaults_capabilities_and_workflows_match_upstream() {
    let data = fixtures();
    for case in data["defaults"].as_array().unwrap() {
        assert_eq!(
            json!(get_video_defaults(case["id"].as_str().unwrap())),
            case["expected"],
            "{case}"
        );
    }
    for case in data["editModels"].as_array().unwrap() {
        assert_eq!(
            json!(is_edit_image_model(case["id"].as_str().unwrap())),
            case["expected"],
            "{case}"
        );
    }
    for case in data["workflows"].as_array().unwrap() {
        let actual = filter_video_models_by_workflow(
            data["models"].as_array().unwrap(),
            &[case["workflow"].as_str().unwrap()],
        );
        assert_eq!(json!(actual), case["expected"], "{case}");
    }
}

#[test]
fn bundled_hosted_schemas_match_all_upstream_fingerprints() {
    let schemas = fixtures()["schemas"].as_array().unwrap().clone();
    assert_eq!(schemas.len(), crate::chat::HOSTED_TOOL_NAMES.len());
    for schema in schemas {
        let tool = crate::chat::HostedTools
            .get(schema["name"].as_str().unwrap())
            .unwrap();
        // serde_json maps use sorted keys, matching the fixture generator.
        let bytes = serde_json::to_vec(&tool["function"]["parameters"]).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            schema["sha256"].as_str().unwrap(),
            "{}",
            schema["name"]
        );
    }
    for name in ["generate_speech", "upscale_image", "upscale_video"] {
        assert!(crate::chat::HostedTools.get(name).is_some());
    }
}

#[test]
fn selection_prioritizes_compatible_requests_preferences_then_stable_worker_order() {
    let models = vec![
        json!({"id":"ltx23-22b-fp8_t2v_distilled","media":"video","workerCount":1}),
        json!({"id":"wan_v2.2-14b-fp8_t2v_lightx2v","media":"video","workerCount":10}),
        json!({"id":"wan_v2.2-14b-fp8_i2v_lightx2v","media":"video","workerCount":20}),
        json!({"id":"ltx25-22b-int8_t2v_distilled","media":"video","workerCount":10}),
    ];
    let mut options = BackboneModelOptions::new("video");
    options.workflows = Some(&["t2v"]);
    let selected = select_backbone_model(&models, &options).unwrap();
    assert_eq!(selected.model_id, "wan_v2.2-14b-fp8_t2v_lightx2v");
    assert_eq!(
        selected.selected_by,
        BackboneModelSelectionReason::WorkerCount
    );
    options.preferred_model_ids = &["ltx23-22b-fp8_t2v_distilled"];
    assert_eq!(
        select_backbone_model(&models, &options)
            .unwrap()
            .selected_by,
        BackboneModelSelectionReason::PreferredModel
    );
    options.requested_model = Some("ltx25-22b-int8_t2v_distilled");
    assert_eq!(
        select_backbone_model(&models, &options)
            .unwrap()
            .selected_by,
        BackboneModelSelectionReason::RequestedModel
    );
    options.requested_model = Some("wan_v2.2-14b-fp8_i2v_lightx2v");
    assert_eq!(
        select_backbone_model(&models, &options).unwrap().model_id,
        "ltx23-22b-fp8_t2v_distilled"
    );
    options.workflows = Some(&["flfa2v"]);
    assert!(
        select_backbone_model(&models, &options)
            .unwrap_err()
            .to_string()
            .contains("workflows: flfa2v")
    );
}
