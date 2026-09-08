use super::*;
pub(super) fn build_image_keyframe(
    params: &Map<String, Value>,
    options: &ModelOptions,
    keyframe: &mut Map<String, Value>,
) -> Result<()> {
    let model_id = required_str(params, "modelId")?;
    let comfy = [
        "z_image_",
        "dark_beast_z_image_",
        "krea2_",
        "dark_beast_krea2_",
        "qwen_image_",
        "wan_",
        "ace_step",
        "rtx_vsr_",
        "minimax_music3",
    ]
    .iter()
    .any(|prefix| model_id.starts_with(prefix));
    let sampler = validate_option(params.get("sampler"), options.raw.get("sampler"), "sampler")?;
    let scheduler = validate_option(
        params.get("scheduler"),
        options.raw.get("scheduler"),
        "scheduler",
    )?;
    if let Some(sampler) = sampler {
        keyframe.insert(
            if comfy { "comfySampler" } else { "scheduler" }.into(),
            sampler,
        );
    } else if comfy {
        keyframe.insert("comfySampler".into(), Value::Null);
    }
    if let Some(scheduler) = scheduler {
        keyframe.insert(
            if comfy {
                "comfyScheduler"
            } else {
                "timeStepSpacing"
            }
            .into(),
            scheduler,
        );
    } else if comfy {
        keyframe.insert("comfyScheduler".into(), Value::Null);
    }
    if comfy {
        let vae = validate_option(params.get("vae"), options.raw.get("vae"), "vae")?;
        keyframe.insert("vae".into(), vae.unwrap_or(Value::Null));
    }
    let has_starting_image = truthy(params.get("startingImage"));
    if has_starting_image {
        let strength = number(params.get("startingImageStrength")).unwrap_or(0.5);
        if !(0.0..=1.0).contains(&strength) {
            return Err(Error::InvalidInput(
                "startingImageStrength must be between 0 and 1".into(),
            ));
        }
        keyframe.insert("hasStartingImage".into(), json!(true));
        keyframe.insert("strengthIsEnabled".into(), json!(true));
        keyframe.insert("strength".into(), json!(1.0 - strength));
    }
    if model_id == SAM3_MODEL_ID {
        if !has_starting_image {
            return Err(Error::InvalidInput(
                "SAM3 image segmentation requires startingImage".into(),
            ));
        }
        let prompt = params
            .get("sam3Prompt")
            .filter(|v| !sam3::is_falsy(v))
            .ok_or_else(|| {
                Error::InvalidInput("SAM3 image segmentation requires sam3Prompt".into())
            })?;
        keyframe.insert("sam3Prompt".into(), normalize_sam3_prompt(prompt)?);
    } else if params.contains_key("sam3Prompt") {
        return Err(Error::InvalidInput(format!(
            "sam3Prompt is only supported by {SAM3_MODEL_ID}"
        )));
    }
    if model_id == PIXAL3D_MODEL_ID && !has_starting_image {
        return Err(Error::InvalidInput(
            "Pixal3D reconstruction requires startingImage".into(),
        ));
    }
    for index in 1_usize..=16 {
        let direct = truthy(params.get(&format!("contextImage{index}")));
        let array = params
            .get("contextImages")
            .and_then(Value::as_array)
            .is_some_and(|images| images.get(index - 1).is_some_and(truthy_value));
        keyframe.insert(format!("hasContextImage{index}"), json!(direct || array));
    }
    let mut size = params
        .get("sizePreset")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    if params.get("width").is_some() && params.get("height").is_some() && size.is_none() {
        size = Some("custom".into());
    }
    if let Some(size) = size {
        keyframe.insert("sizePreset".into(), json!(size));
        if size == "custom" {
            let (minimum, maximum) = custom_image_size_bounds(model_id);
            let width = ranged_number(params.get("width"), "width", minimum, maximum)?;
            let height = ranged_number(params.get("height"), "height", minimum, maximum)?;
            keyframe.insert("width".into(), json!(width));
            keyframe.insert("height".into(), json!(height));
        }
    }
    if let Some(control) = params.get("controlNet").and_then(Value::as_object) {
        let name = control
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("controlNet.name is required".into()))?;
        let mut raw = json!({
            "name": name,
            "cnImageState": "original",
            "hasImage": truthy(control.get("image")),
        });
        for (source, target) in [
            ("strength", "controlStrength"),
            ("guidanceStart", "controlGuidanceStart"),
            ("guidanceEnd", "controlGuidanceEnd"),
        ] {
            if let Some(value) = control.get(source) {
                let value = ranged_number(Some(value), source, 0.0, 1.0)?;
                raw[target] = json!(value);
            }
        }
        if let Some(mode) = control.get("mode").and_then(Value::as_str) {
            raw["controlMode"] = json!(match mode {
                "balanced" => 0,
                "prompt_priority" => 1,
                "cn_priority" => 2,
                _ => {
                    return Err(Error::InvalidInput(format!(
                        "unsupported controlNet.mode {mode}"
                    )));
                }
            });
        }
        keyframe.insert("currentControlNetsJob".into(), json!([raw]));
    }
    copy_if_present(params, keyframe, "gptImageQuality", "gptImageQuality");
    copy_if_present(params, keyframe, "gptImageBackground", "gptImageBackground");
    Ok(())
}
