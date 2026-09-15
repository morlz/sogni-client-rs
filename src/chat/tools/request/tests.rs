use super::*;

#[test]
fn direct_tool_requests_match_fresh_typescript_builders() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../fixtures/upstream-tool-requests.json")).unwrap();
    for case in fixtures["requests"].as_array().unwrap() {
        let plan = plan_tool_request(
            case["tool"].as_str().unwrap(),
            &case["args"],
            &case["options"],
            fixtures["models"].as_array().unwrap(),
        )
        .unwrap();
        // JSON treats integer and floating number representations alike.
        fn normalize(value: &mut Value) {
            match value {
                Value::Number(number) if number.as_f64().is_some_and(|n| n.fract() == 0.0) => {
                    *value = json!(number.as_f64().unwrap() as i64)
                }
                Value::Array(array) => array.iter_mut().for_each(normalize),
                Value::Object(object) => object.values_mut().for_each(normalize),
                _ => {}
            }
        }
        let mut actual = Value::Object(plan.params);
        let mut expected = case["expected"].clone();
        normalize(&mut actual);
        normalize(&mut expected);
        assert_eq!(actual, expected, "{}: {}", case["tool"], case["args"]);
    }
}

#[test]
fn h3_audio_modes_and_indexed_references_do_not_fall_back_silently() {
    let models = vec![
        json!({"id":"minimax-h3-fastvideo-int8_flfa2v_turbo","media":"video"}),
        json!({"id":"ltx23-22b-fp8_a2v_distilled","media":"video"}),
    ];
    let args = json!({"videoModel":"minimax-h3-fasth3-flfa2v-turbo","prompt":"a song","reference_audio_url":"data:audio/wav;base64,UklGRgAAAABXQVZF"});
    assert!(
        plan_tool_request("sound_to_video", &args, &Value::Null, &models)
            .unwrap_err()
            .to_string()
            .contains("needs reference_image_url and reference_image_end_url")
    );
    assert!(
        plan_tool_request(
            "generate_video",
            &json!({"referenceImageIndices":[0]}),
            &Value::Null,
            &models
        )
        .unwrap_err()
        .to_string()
        .contains("mediaContext")
    );
    for tool in ["generate_speech", "upscale_image", "upscale_video"] {
        assert!(
            plan_tool_request(tool, &json!({}), &Value::Null, &models)
                .unwrap_err()
                .to_string()
                .contains("hosted chat")
        );
    }
}
