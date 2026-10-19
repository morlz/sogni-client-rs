use super::*;

const FLUX: &str = "flux3_image_klein_sd_int8_convrot";
const MOSS: [&str; 2] = [
    "moss_image_sft_int8_convrot",
    "moss_image_sft_dmd8_int8_convrot",
];

fn wire(request: &ProjectRequest) -> Result<Value> {
    build_job_request(
        "UPSTREAM-5-66",
        &request.params,
        &ModelOptions {
            model_id: request.params["modelId"].as_str().unwrap().into(),
            media_type: "image".into(),
            raw: json!({}),
        },
        None,
    )
}

fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if number.as_f64().is_some_and(|n| n.fract() == 0.0) => {
            *value = json!(number.as_f64().unwrap() as i64);
        }
        Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
        Value::Object(values) => values.values_mut().for_each(normalize_numbers),
        _ => {}
    }
}

#[test]
fn moss_and_flux_image_wires_and_refusals_match_typescript_5_66() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/upstream-5-66.json")).unwrap();
    assert_eq!(fixtures["version"], "5.66.0");
    assert_eq!(
        fixtures["commit"],
        "9805a8c146dea25065d080881e07da4c5c878a88"
    );
    assert_eq!(fixtures["protocolVersion"], "1.0.0-alpha.52");
    for (field, refused) in [("cases", false), ("invalid", true)] {
        for case in fixtures[field].as_array().unwrap() {
            let options = ModelOptions {
                model_id: case["params"]["modelId"].as_str().unwrap().into(),
                media_type: "image".into(),
                raw: case["options"].clone(),
            };
            let result = build_job_request(
                "UPSTREAM-5-66",
                case["params"].as_object().unwrap(),
                &options,
                None,
            );
            if refused {
                let expected = case["message"].as_str().unwrap();
                let Error::InvalidInput(actual) = result.unwrap_err() else {
                    panic!("{} must fail input validation", case["name"]);
                };
                let lower = expected.to_lowercase();
                if lower.starts_with("width must ") || lower.starts_with("height must ") {
                    // Rust retains its established inclusive-range wording.
                    // The field and rejected bound still come from TypeScript.
                    let field = lower.split_whitespace().next().unwrap();
                    let bound = lower
                        .split(", got ")
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .last()
                        .unwrap();
                    assert!(
                        actual.starts_with(&format!("{field} must be between ")),
                        "{}: {actual}",
                        case["name"]
                    );
                    assert!(
                        actual.split_whitespace().any(|word| word == bound),
                        "{}: {actual}; TypeScript: {expected}",
                        case["name"]
                    );
                } else {
                    assert_eq!(actual, expected, "{}", case["name"]);
                }
            } else {
                let mut actual = result.unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
                let mut expected = case["wire"].clone();
                normalize_numbers(&mut actual);
                normalize_numbers(&mut expected);
                assert_eq!(actual, expected, "{}", case["name"]);
            }
        }
    }
    for capability in fixtures["capabilities"].as_array().unwrap() {
        assert_eq!(
            max_context_images(capability["modelId"].as_str().unwrap()) as u64,
            capability["maxContextImages"].as_u64().unwrap()
        );
    }
}

#[test]
fn flux_public_create_limit_matches_upload_admission_rather_than_bare_serializer() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/upstream-5-66.json")).unwrap();
    // Upstream's serializer exposes 16 generic flags. Its public image upload
    // admission separately enforces the selected model's ten-reference limit.
    assert_eq!(
        fixtures["lowLevelOverflow"]["wire"]["keyFrames"][0]["hasContextImage11"],
        true
    );
    for case in fixtures["admission"].as_array().unwrap() {
        let request = ProjectRequest::from_value(case["params"].clone()).unwrap();
        let actual = wire(&request);
        if case["accepted"] == true {
            assert!(actual.is_ok(), "{}: {actual:?}", case["name"]);
            let count = case["params"]["contextImages"].as_array().unwrap().len();
            assert_eq!(case["uploadedSlots"], json!((0..count).collect::<Vec<_>>()));
        } else {
            assert!(
                actual
                    .unwrap_err()
                    .to_string()
                    .contains(case["message"].as_str().unwrap())
            );
            assert_eq!(case["uploadedSlots"], json!([]));
        }
    }
}

#[test]
fn moss_scales_preserve_zero_omit_neutral_and_reject_non_finite_inputs() {
    for model in MOSS {
        let request = ProjectRequest::image(model, "{\"caption\":\"exact 山海\"}")
            .dimensions(513, 2047)
            .moss_rope_scale_x(0.0)
            .moss_rope_scale_y(1.0);
        let result = wire(&request).unwrap();
        let frame = &result["keyFrames"][0];
        assert_eq!(frame["mossRopeScaleX"], 0.0);
        assert!(frame.get("mossRopeScaleY").is_none());
        assert_eq!(frame["width"], 513.0);
        assert_eq!(frame["height"], 2047.0);
        assert_eq!(frame["positivePrompt"], request.params["positivePrompt"]);
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for request in [
                ProjectRequest::image(model, "prompt").moss_rope_scale_x(value),
                ProjectRequest::image(model, "prompt").moss_rope_scale_y(value),
            ] {
                assert!(
                    wire(&request)
                        .unwrap_err()
                        .to_string()
                        .contains("must be a finite number between 0 and 100")
                );
            }
        }
    }
}

#[test]
fn new_image_reference_limits_include_numbered_assets_without_double_counting() {
    let mut request = ProjectRequest::image(FLUX, "all ten references")
        .param("contextImages", json!(vec![true; 10]));
    for slot in 1..=10 {
        request = request.asset(AssetRole::ContextImage(slot), MediaSource::bytes(vec![0]));
    }
    let result = wire(&request).unwrap();
    for slot in 1..=16 {
        assert_eq!(
            result["keyFrames"][0][format!("hasContextImage{slot}")],
            slot <= 10
        );
    }
    let overflow = request.asset(AssetRole::ContextImage(11), MediaSource::bytes(vec![0]));
    assert!(
        wire(&overflow)
            .unwrap_err()
            .to_string()
            .contains("Up to 10 context images")
    );
    for model in MOSS {
        for role in [
            AssetRole::StartingImage,
            AssetRole::ContextImage(1),
            AssetRole::ContextImage(16),
        ] {
            let request =
                ProjectRequest::image(model, "text only").asset(role, MediaSource::bytes(vec![0]));
            assert!(
                wire(&request)
                    .unwrap_err()
                    .to_string()
                    .contains("does not accept guide or reference images")
            );
        }
    }
    // Existing serializers keep their prior generic 16-slot behavior.
    assert!(
        wire(
            &ProjectRequest::image("qwen_image_edit_2511_fp8", "unchanged")
                .param("contextImages", json!(vec![true; 16]))
        )
        .is_ok()
    );
}
