use super::*;

#[test]
fn complete_utility_requests_match_upstream_5_50_serializer() {
    // Captured from the unmodified TypeScript serializer at 452e789, including
    // worker reset fields. Fixtures include their exact source parameters.
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/utility-contract.json")).unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let options = ModelOptions {
            model_id: case["params"]["modelId"].as_str().unwrap().into(),
            media_type: case["options"]["type"].as_str().unwrap().into(),
            raw: case["options"].clone(),
        };
        let mut actual = build_job_request(
            "UPSTREAM-FIXTURE",
            case["params"].as_object().unwrap(),
            &options,
            None,
        )
        .unwrap();
        let mut expected = case["wire"].clone();
        normalize_numbers(&mut actual);
        normalize_numbers(&mut expected);
        pretty_assertions::assert_eq!(actual, expected, "upstream case {}", case["name"]);
    }
}

#[test]
fn generation_requests_and_rejections_match_upstream_5_50_serializer() {
    // Includes all six FastH3 modes, speech, ordered GPT references and masks,
    // FlashVSR source timing, Pixal3D views, SAM3 selections and Seedance exports.
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/generation-contract.json")).unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let options = fixture_options(case);
        let mut actual = build_job_request(
            "UPSTREAM-FIXTURE",
            case["params"].as_object().unwrap(),
            &options,
            None,
        )
        .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
        let mut expected = case["wire"].clone();
        normalize_numbers(&mut actual);
        normalize_numbers(&mut expected);
        pretty_assertions::assert_eq!(actual, expected, "upstream case {}", case["name"]);
    }
    for case in fixtures["invalid"].as_array().unwrap() {
        let options = fixture_options(case);
        let error = build_job_request(
            "UPSTREAM-FIXTURE",
            case["params"].as_object().unwrap(),
            &options,
            None,
        )
        .unwrap_err();
        let message = case["message"].as_str().unwrap().trim_end_matches('.');
        assert!(
            error.to_string().contains(message),
            "{}: {error}; upstream: {message}",
            case["name"]
        );
    }
}

#[test]
fn speech_and_upscale_capabilities_match_upstream_tier_mapping() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/generation-contract.json")).unwrap();
    for fixture in fixtures["tiers"].as_array().unwrap() {
        let mut actual = map_model_options(&fixture["tier"], fixture["media"].as_str().unwrap());
        let mut expected = fixture["options"].clone();
        normalize_numbers(&mut actual);
        normalize_numbers(&mut expected);
        pretty_assertions::assert_eq!(actual, expected, "upstream tier {}", fixture["name"]);
    }
}

fn fixture_options(case: &Value) -> ModelOptions {
    ModelOptions {
        model_id: case["params"]["modelId"].as_str().unwrap().into(),
        media_type: case["options"]["type"].as_str().unwrap().into(),
        raw: case["options"].clone(),
    }
}

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
        Value::Object(values) => values.values_mut().for_each(normalize_numbers),
        Value::Number(number) => *value = json!(number.as_f64().unwrap()),
        _ => {}
    }
}
