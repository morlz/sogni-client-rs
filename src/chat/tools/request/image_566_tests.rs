use super::*;

#[test]
fn image_edit_reference_refusals_match_typescript_and_never_drop_inputs() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../projects/wire/fixtures/upstream-5-66.json"
    ))
    .unwrap();
    let models = fixture["toolModels"].as_array().unwrap();
    for case in fixture["toolRefusals"].as_array().unwrap() {
        let error = plan("edit_image", &case["args"], &json!({}), models).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(case["message"].as_str().unwrap()),
            "{case}: {error}"
        );
    }
    let mut args = fixture["toolRefusals"][0]["args"].clone();
    args["reference_image_urls"].as_array_mut().unwrap().pop();
    let plan = plan("edit_image", &args, &json!({}), models).unwrap();
    assert_eq!(plan.params["modelId"], "flux3_image_klein_sd_int8_convrot");
    assert_eq!(plan.assets.len(), 10);
    assert_eq!(plan.params["contextImages"], json!(vec![true; 10]));
    for (index, asset) in plan.assets.iter().enumerate() {
        assert_eq!(asset.role, AssetRole::ContextImage((index + 1) as u8));
    }
}
