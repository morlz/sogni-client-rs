use super::*;
fn normalize(value: &mut Value) {
    match value {
        Value::Number(n) if n.as_f64().is_some_and(|n| n.fract() == 0.0) => {
            *value = json!(n.as_f64().unwrap() as i64)
        }
        Value::Array(values) => values.iter_mut().for_each(normalize),
        Value::Object(values) => values.values_mut().for_each(normalize),
        _ => {}
    }
}
#[test]
fn generation_5_72_compression_and_seedream_output_match_upstream() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/upstream-5-72.json")).unwrap();
    assert_eq!(fixture["version"], "5.72.0");
    assert_eq!(
        fixture["commit"],
        "0e5e31075fdeea276a4241c924bd3423c60804ce"
    );
    for case in fixture["cases"].as_array().unwrap() {
        let options = ModelOptions {
            model_id: case["params"]["modelId"].as_str().unwrap().into(),
            media_type: case["params"]["type"].as_str().unwrap().into(),
            raw: case["options"].clone(),
        };
        let mut actual = build_job_request(
            "UPSTREAM-5-72",
            case["params"].as_object().unwrap(),
            &options,
            None,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        let mut expected = case["wire"].clone();
        normalize(&mut actual);
        normalize(&mut expected);
        assert_eq!(actual, expected, "{}", case["name"]);
    }
    for case in fixture["invalid"].as_array().unwrap() {
        let options = ModelOptions {
            model_id: case["params"]["modelId"].as_str().unwrap().into(),
            media_type: "video".into(),
            raw: case["options"].clone(),
        };
        let Error::InvalidInput(message) = build_job_request(
            "UPSTREAM-5-72",
            case["params"].as_object().unwrap(),
            &options,
            None,
        )
        .unwrap_err() else {
            panic!("expected invalid input")
        };
        assert_eq!(
            message.trim_end_matches('.'),
            case["message"].as_str().unwrap().trim_end_matches('.'),
            "{}",
            case["name"]
        );
    }
}
