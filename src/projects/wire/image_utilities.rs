use super::*;

pub(super) fn apply_utility_fields(
    params: &Map<String, Value>,
    model_id: &str,
    keyframe: &mut Map<String, Value>,
) -> Result<()> {
    if model_id == BIREFNET_BACKGROUND_REMOVAL_MODEL_ID {
        if !truthy(params.get("startingImage")) {
            return Err(Error::InvalidInput(
                "BiRefNet background removal requires startingImage".into(),
            ));
        }
        let apply_mask = params
            .get("applyMask")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| Error::InvalidInput("applyMask must be a boolean".into()))
            })
            .transpose()?
            .unwrap_or(false);
        keyframe.insert("applyMask".into(), json!(apply_mask));
    } else if params.contains_key("applyMask") {
        return Err(Error::InvalidInput(format!(
            "applyMask is only supported by {BIREFNET_BACKGROUND_REMOVAL_MODEL_ID}"
        )));
    }
    let single = PIXAL3D_IMAGE_TO_3D_MODEL_ID;
    let multi = PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID;
    if is_pixal3d_model(model_id) {
        if !truthy(params.get("startingImage")) {
            let message = if is_pixal3d_multi_view_model(model_id) {
                "Pixal3D multi-view reconstruction requires startingImage (the front view)"
            } else {
                "Pixal3D reconstruction requires startingImage"
            };
            return Err(Error::InvalidInput(message.into()));
        }
        let generic_context = params
            .get("contextImages")
            .and_then(Value::as_array)
            .is_some_and(|images| images.iter().any(truthy_value))
            || (1..=16).any(|slot| truthy(params.get(&format!("contextImage{slot}"))));
        if generic_context {
            return Err(Error::InvalidInput(
                if is_pixal3d_multi_view_model(model_id) {
                    format!(
                        "{multi} takes its orbit views as leftViewImage, backViewImage and rightViewImage, not contextImages"
                    )
                } else {
                    format!(
                        "{single} reconstructs from startingImage alone and does not support contextImages; use {multi} for more views"
                    )
                },
            ));
        }
    }
    // Named views carry their own fixed upload slot. Do not compact subsets.
    for (view, slot) in PIXAL3D_ORBIT_VIEW_SLOTS {
        if let Some(value) = params.get(view) {
            if !is_pixal3d_multi_view_model(model_id) {
                return Err(Error::InvalidInput(if model_id == single {
                    format!(
                        "{single} reconstructs from startingImage alone and ignores {view}; use {multi} for orbit views"
                    )
                } else {
                    format!("{view} is only supported by {multi}")
                }));
            }
            if !truthy_value(value) {
                return Err(Error::InvalidInput(format!(
                    "{view} must be an image; leave it unset to omit that view"
                )));
            }
            keyframe.insert(format!("hasContextImage{slot}"), json!(true));
        }
    }
    if let Some(variant) = params.get("templateVariant") {
        if model_id != single {
            return Err(Error::InvalidInput(format!(
                "templateVariant is only supported by {single}"
            )));
        }
        if variant.as_str() != Some("i23d-birefnet") {
            return Err(Error::InvalidInput(
                "templateVariant must be one of: i23d-birefnet".into(),
            ));
        }
        keyframe.insert("templateVariant".into(), variant.clone());
    }
    // These bounds describe the public request controls. The service validates
    // and prices them independently; omission retains its model defaults.
    for (field, min, max) in [
        ("textureSize", 1024, 4096),
        ("meshTargetFaces", 5000, 700000),
        ("normalMapSize", 512, 2048),
        ("ambientOcclusionSize", 256, 1024),
        ("shapeResolution", 1024, 1536),
    ] {
        if let Some(value) = params.get(field) {
            if !is_pixal3d_model(model_id) {
                return Err(Error::InvalidInput(format!(
                    "{field} is only supported by {single} and {multi}"
                )));
            }
            if !value
                .as_u64()
                .is_some_and(|value| (min..=max).contains(&value))
            {
                return Err(Error::InvalidInput(format!(
                    "{field} must be an integer from {min} to {max}"
                )));
            }
            keyframe.insert(field.into(), value.clone());
        }
    }
    Ok(())
}
