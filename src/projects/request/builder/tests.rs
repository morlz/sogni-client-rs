use super::*;

#[test]
fn pixal_orbit_assets_keep_fixed_slots_when_views_are_omitted() {
    let request = ProjectRequest::image(PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID, "")
        .asset(AssetRole::StartingImage, MediaSource::bytes("front"))
        .asset(AssetRole::Pixal3dBackView, MediaSource::bytes("back"))
        .asset(AssetRole::Pixal3dRightView, MediaSource::bytes("right"));
    validate_asset_roles(PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID, &request.assets).unwrap();
    assert_eq!(
        request
            .assets
            .iter()
            .map(|(role, _)| effective_asset_wire_name(
                role,
                PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID
            ))
            .collect::<Vec<_>>(),
        ["startingImage", "contextImage2", "contextImage3"]
    );
    assert_eq!(request.params.get("leftViewImage"), None);
    assert_eq!(request.params["backViewImage"], true);
    assert_eq!(request.params["rightViewImage"], true);
    let options = ModelOptions {
        model_id: PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID.into(),
        media_type: "image".into(),
        raw: json!({}),
    };
    let wire = build_job_request("VIEWS", &request.params, &options, None).unwrap();
    assert_eq!(wire["keyFrames"][0]["hasContextImage1"], false);
    assert_eq!(wire["keyFrames"][0]["hasContextImage2"], true);
    assert_eq!(wire["keyFrames"][0]["hasContextImage3"], true);
}

#[test]
fn orbit_and_generic_context_cannot_claim_the_same_upload_slot() {
    let request = ProjectRequest::image(PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID, "")
        .asset(AssetRole::Pixal3dRightView, MediaSource::bytes("right"))
        .asset(AssetRole::ContextImage(3), MediaSource::bytes("duplicate"));
    let error =
        validate_asset_roles(PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID, &request.assets).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("duplicate project asset role contextImage3")
    );
}

#[test]
fn gpt_mask_role_marks_a_png_reference_mask_and_requires_a_first_reference() {
    let request = ProjectRequest::image("gpt-image-2.5-flare", "Edit this image.").asset(
        AssetRole::GptImageMask,
        MediaSource::named_bytes("mask", "mask.png", "image/png"),
    );
    assert_eq!(
        effective_asset_wire_name(&AssetRole::GptImageMask, "gpt-image-2.5-flare"),
        "referenceMask"
    );
    assert_eq!(request.params["gptImageMask"], true);
    assert!(!request.params.contains_key("referenceMask"));
    let options = ModelOptions {
        model_id: "gpt-image-2.5-flare".into(),
        media_type: "image".into(),
        raw: json!({}),
    };
    assert!(
        build_job_request("MASK", &request.params, &options, None)
            .unwrap_err()
            .to_string()
            .contains("requires a first reference image")
    );
    let request = request.asset(AssetRole::ContextImage(1), MediaSource::bytes("source"));
    let wire = build_job_request("MASK", &request.params, &options, None).unwrap();
    assert_eq!(wire["keyFrames"][0]["hasReferenceMask"], true);
    assert_eq!(
        wire["keyFrames"][0]["referenceMaskContentType"],
        "image/png"
    );
    assert_eq!(wire["keyFrames"][0]["hasContextImage1"], true);
}
