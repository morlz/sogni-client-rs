use super::*;

#[test]
fn generation_586_matches_compiled_typescript_fields_and_refusals() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/upstream-5-86.json")).unwrap();
    assert_eq!(fixture["version"], "5.86.2");
    assert_eq!(
        fixture["commit"],
        "e830ed42e6b6973e659aa98c226b05f864e6ea97"
    );
    for case in fixture["cases"].as_array().unwrap() {
        let options = ModelOptions {
            model_id: case["params"]["modelId"].as_str().unwrap().into(),
            media_type: "video".into(),
            raw: case["options"].clone(),
        };
        let actual = build_job_request(
            "UPSTREAM-5-86",
            case["params"].as_object().unwrap(),
            &options,
            None,
        )
        .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
        for (field, expected) in case["wire"]["keyFrames"][0].as_object().unwrap() {
            if let Some(number) = expected.as_f64() {
                assert_eq!(
                    actual["keyFrames"][0][field].as_f64(),
                    Some(number),
                    "{}: {field}",
                    case["name"]
                );
            } else {
                assert_eq!(
                    &actual["keyFrames"][0][field], expected,
                    "{}: {field}",
                    case["name"]
                );
            }
        }
        assert_eq!(
            actual["keyFrames"][0].get("seedanceDraftFinalResolution"),
            case["wire"]["keyFrames"][0].get("seedanceDraftFinalResolution"),
            "{}",
            case["name"]
        );
    }
    for case in fixture["refusals"].as_array().unwrap() {
        let options = ModelOptions {
            model_id: case["params"]["modelId"].as_str().unwrap().into(),
            media_type: "video".into(),
            raw: case["options"].clone(),
        };
        let error = build_job_request(
            "UPSTREAM-5-86",
            case["params"].as_object().unwrap(),
            &options,
            None,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(case["message"].as_str().unwrap()),
            "{}: {error}",
            case["name"]
        );
    }
    for case in fixture["models"].as_array().unwrap() {
        let model = case["model"].as_str().unwrap();
        assert_eq!(json!(is_video_model(model)), case["video"]);
        assert_eq!(json!(is_minimax_h3_model(model)), case["h3"]);
        assert_eq!(
            json!(crate::is_minimax_h3_prism_model(model)),
            case["prism"]
        );
        assert_eq!(
            json!(crate::is_minimax_h3_keyframe_model(model)),
            case["keyframes"]
        );
        assert_eq!(json!(get_video_workflow_type(model)), case["workflow"]);
    }
}
