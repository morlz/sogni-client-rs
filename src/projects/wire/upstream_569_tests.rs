use super::*;

fn normalize(value: &mut Value) {
    match value {
        Value::Number(number) if number.as_f64().is_some_and(|n| n.fract() == 0.0) => {
            *value = json!(number.as_f64().unwrap() as i64);
        }
        Value::Array(values) => values.iter_mut().for_each(normalize),
        Value::Object(values) => values.values_mut().for_each(normalize),
        _ => {}
    }
}

#[test]
fn generation_wires_and_refusals_match_typescript_5_69() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/upstream-5-69.json")).unwrap();
    assert_eq!(fixtures["version"], "5.69.1");
    assert_eq!(
        fixtures["commit"],
        "5970a1bd40b88f678719188969c8633bc313b665"
    );
    assert_eq!(fixtures["protocolVersion"], "1.0.0-alpha.55");
    for case in fixtures["cases"].as_array().unwrap() {
        let options = ModelOptions {
            model_id: case["params"]["modelId"].as_str().unwrap().into(),
            media_type: case["params"]["type"].as_str().unwrap().into(),
            raw: case["options"].clone(),
        };
        let mut actual = build_job_request(
            "UPSTREAM-5-69",
            case["params"].as_object().unwrap(),
            &options,
            None,
        )
        .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
        let mut expected = case["wire"].clone();
        normalize(&mut actual);
        normalize(&mut expected);
        assert_eq!(actual, expected, "{}", case["name"]);
    }
    for case in fixtures["invalid"].as_array().unwrap() {
        let options = ModelOptions {
            model_id: case["params"]["modelId"].as_str().unwrap().into(),
            media_type: case["params"]["type"].as_str().unwrap().into(),
            raw: case["options"].clone(),
        };
        let Error::InvalidInput(actual) = build_job_request(
            "UPSTREAM-5-69",
            case["params"].as_object().unwrap(),
            &options,
            None,
        )
        .unwrap_err() else {
            panic!("{} must fail input validation", case["name"])
        };
        let expected = case["message"].as_str().unwrap();
        let lower = expected.to_ascii_lowercase();
        if lower.starts_with("width must ") || lower.starts_with("height must ") {
            let field = lower.split_whitespace().next().unwrap();
            let bound = lower
                .split(", got ")
                .next()
                .unwrap()
                .split_whitespace()
                .last()
                .unwrap();
            assert!(
                actual.starts_with(&format!("{field} must be between ")) && actual.contains(bound),
                "{}: {actual}",
                case["name"]
            );
        } else if case["name"] == "extend-external-url" {
            // Keep the established Rust capitalization of this shared refusal.
            assert!(
                actual
                    .trim_end_matches('.')
                    .eq_ignore_ascii_case(expected.trim_end_matches('.'))
            );
        } else if case["name"].as_str().unwrap().ends_with("-missing-visual") {
            assert!(actual.contains("visual reference"), "{actual}");
        } else {
            assert_eq!(
                actual.trim_end_matches('.'),
                expected.trim_end_matches('.'),
                "{}",
                case["name"]
            );
        }
    }
}

#[test]
fn h3_predicates_and_model_specific_assets_match_upstream() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/upstream-5-69.json")).unwrap();
    for case in fixtures["predicates"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        assert_eq!(crate::is_minimax_h3_extend_model(id), case["extend"]);
        assert_eq!(
            crate::is_minimax_h3_fasth3_reference_model(id),
            case["fasth3Reference"]
        );
        assert_eq!(crate::is_minimax_h3_turbo_model(id), case["turbo"]);
        assert_eq!(crate::is_minimax_h3_keyframe_model(id), case["keyframes"]);
        let workflow = get_video_workflow_type(id).unwrap();
        assert_eq!(
            json!(video_asset_requirements(id, workflow)),
            case["assets"],
            "{id}"
        );
    }
}
