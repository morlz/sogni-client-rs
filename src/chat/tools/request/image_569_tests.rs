use super::*;

#[test]
fn seedream_tool_requests_match_typescript_and_preserve_explicit_selection() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../projects/wire/fixtures/upstream-5-69.json"
    ))
    .unwrap();
    let models = fixtures["models"].as_array().unwrap();
    for case in fixtures["toolCases"].as_array().unwrap() {
        let plan = plan(
            case["tool"].as_str().unwrap(),
            &case["args"],
            &json!({}),
            models,
        )
        .unwrap();
        assert_eq!(
            Value::Object(plan.params),
            case["expected"],
            "{}",
            case["args"]
        );
    }
    let seedream = models[..1].to_vec();
    let error = plan(
        "generate_image",
        &json!({"prompt":"a portrait"}),
        &json!({}),
        &seedream,
    )
    .unwrap_err();
    assert!(error.to_string().contains("No compatible image models"));
    let overflow = plan("edit_image",&json!({"model":crate::SEEDREAM_5_PRO_UNCENSORED_MODEL_ID,"prompt":"an edit","source_image_url":"data:image/png;base64,x","reference_image_urls":vec!["data:image/png;base64,x";10]}),&json!({}),models).unwrap_err();
    assert!(
        overflow
            .to_string()
            .contains("accepts at most 10 reference images; this request supplies 11")
    );
}
