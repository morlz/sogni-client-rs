use super::*;

#[test]
fn canonical_template_distinguishes_required_nulls_from_unset_options() {
    let request = ProjectRequest::image("flux1-schnell-fp8", "Synthetic parity fixture")
        .steps(5)
        .guidance(1.0)
        .dimensions(1024, 1024)
        .param("startingImage", true)
        .param("startingImageStrength", 0.75)
        .param("tokenType", Value::Null);
    let options = ModelOptions {
        model_id: "flux1-schnell-fp8".into(),
        media_type: "image".into(),
        raw: json!({"sampler":{"allowed":["euler"],"default":"euler"},"scheduler":{"allowed":["simple"],"default":"simple"}}),
    };
    let actual = build_job_request(
        "00000000-0000-4000-8000-000000000000",
        &request.params,
        &options,
        None,
    )
    .unwrap();
    let mut expected: Value = serde_json::from_str(include_str!(
        "../../client/tests/fixtures/guide-request.json"
    ))
    .unwrap();
    let frame = expected["keyFrames"][0].as_object_mut().unwrap();
    frame.remove("seed");
    frame.insert("scheduler".into(), Value::Null);
    frame.insert("timeStepSpacing".into(), Value::Null);
    frame.insert("width".into(), json!(1024.0));
    frame.insert("height".into(), json!(1024.0));
    frame.insert("guidanceScale".into(), json!(1.0));
    pretty_assertions::assert_eq!(actual, expected);
}
