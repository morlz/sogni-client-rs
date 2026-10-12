use super::*;

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if number.as_f64().is_some_and(|n| n.fract() == 0.0) => {
            *value = json!(number.as_f64().unwrap() as i64);
        }
        Value::Array(array) => array.iter_mut().for_each(normalize_numbers),
        Value::Object(object) => object.values_mut().for_each(normalize_numbers),
        _ => {}
    }
}

#[test]
fn seedance_and_prompt_expansion_match_upstream_5_63_serializer() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/upstream-5-63.json")).unwrap();
    for (field, rejects) in [("cases", false), ("invalid", true)] {
        for case in fixtures[field].as_array().unwrap() {
            let options = ModelOptions {
                model_id: case["params"]["modelId"].as_str().unwrap().into(),
                media_type: case["params"]["type"].as_str().unwrap().into(),
                raw: case["options"].clone(),
            };
            let result = build_job_request(
                "UPSTREAM-5-63",
                case["params"].as_object().unwrap(),
                &options,
                None,
            );
            if rejects {
                let error = result.unwrap_err();
                let message = case["message"].as_str().unwrap().trim_end_matches('.');
                if message.starts_with("Video duration must ") {
                    // Rust retains its established inclusive-range wording;
                    // the authoritative boundary cases must still be refused.
                    assert!(
                        error.to_string().contains("video duration must be between"),
                        "{}: {error}",
                        case["name"]
                    );
                    continue;
                }
                assert!(
                    error.to_string().contains(message),
                    "{}: {error}; upstream: {}",
                    case["name"],
                    case["message"]
                );
            } else {
                let mut actual = result.unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
                let mut expected = case["wire"].clone();
                normalize_numbers(&mut actual);
                normalize_numbers(&mut expected);
                assert_eq!(actual, expected, "{}", case["name"]);
                assert!(actual["keyFrames"][0].get("promptExpanded").is_none());
                assert_eq!(
                    actual["keyFrames"][0]["positivePrompt"],
                    case["params"]["positivePrompt"]
                );
            }
        }
    }
}
