use super::*;

fn request(control: Value) -> Result<Value> {
    let params = ProjectRequest::image("coreml-cyberrealistic_v70_768", "A lighthouse")
        .steps(20)
        .guidance(7.0)
        .param("seed", 7)
        .param("controlNet", control);
    build_job_request(
        "CONTROLNET-FIXTURE",
        &params.params,
        &ModelOptions {
            model_id: "coreml-cyberrealistic_v70_768".into(),
            media_type: "image".into(),
            raw: json!({}),
        },
        None,
    )
}

#[test]
fn controlnet_preprocess_adds_only_true_and_preserves_the_legacy_request() {
    let control = json!({
        "name":"depth", "image":true, "strength":0.8,
        "mode":"cn_priority", "guidanceStart":0, "guidanceEnd":1,
    });
    let baseline = request(control.clone()).unwrap();
    let mut off = control.clone();
    off["preprocess"] = json!(false);
    assert_eq!(
        serde_json::to_string(&request(off).unwrap()).unwrap(),
        serde_json::to_string(&baseline).unwrap(),
    );
    assert_eq!(
        baseline["keyFrames"][0]["currentControlNetsJob"],
        json!([{
            "name":"depth", "cnImageState":"original", "hasImage":true,
            "controlStrength":0.8, "controlMode":2,
            "controlGuidanceStart":0.0, "controlGuidanceEnd":1.0,
        }]),
    );
    let mut on = control;
    on["preprocess"] = json!(true);
    let mut actual = request(on).unwrap();
    assert_eq!(
        actual["keyFrames"][0]["currentControlNetsJob"][0]["preprocess"],
        true
    );
    actual["keyFrames"][0]["currentControlNetsJob"][0]
        .as_object_mut()
        .unwrap()
        .remove("preprocess");
    assert_eq!(actual, baseline);
    let minimal = request(json!({"name":"openpose","image":true,"preprocess":true})).unwrap();
    assert_eq!(
        minimal["keyFrames"][0]["currentControlNetsJob"],
        json!([{"name":"openpose","cnImageState":"original","hasImage":true,"preprocess":true}]),
    );
}

#[test]
fn controlnet_preprocess_rejects_non_booleans_with_the_public_error() {
    for value in [json!("true"), json!(1), Value::Null, json!([]), json!({})] {
        match request(json!({"name":"depth","image":true,"preprocess":value})).unwrap_err() {
            Error::InvalidInput(message) => {
                assert_eq!(message, "controlNet.preprocess must be a boolean")
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
