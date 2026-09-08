use super::*;

#[test]
fn complete_utility_requests_match_upstream_5_32_serializer() {
    // Captured from the unmodified TypeScript serializer at 18b43c9, including
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

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
        Value::Object(values) => values.values_mut().for_each(normalize_numbers),
        Value::Number(number) => *value = json!(number.as_f64().unwrap()),
        _ => {}
    }
}
