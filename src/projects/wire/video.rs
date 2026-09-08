use super::*;
use crate::projects::validation::media_slot_count;
pub(super) fn build_video_keyframe(
    params: &Map<String, Value>,
    options: &ModelOptions,
    keyframe: &mut Map<String, Value>,
) -> Result<()> {
    let model_id = required_str(params, "modelId")?;
    if !is_video_model(model_id) && options.media_type != "video" {
        return Err(Error::InvalidInput(
            "video generation requires a video model".into(),
        ));
    }
    validate_video_assets(params, model_id)?;
    validate_h3_params(params, model_id)?;
    for (field, target) in [
        ("referenceImage", "hasReferenceImage"),
        ("referenceImageEnd", "hasReferenceImageEnd"),
    ] {
        if truthy(params.get(field)) {
            keyframe.insert(target.into(), json!(true));
        }
    }
    if is_minimax_h3_reference_model(model_id) {
        for slot in
            1..=media_slot_count(params.get("referenceAudio"), params.get("referenceAudios"))
        {
            keyframe.insert(format!("hasReferenceAudio{slot}"), json!(true));
        }
        let durations = params
            .get("referenceVideoDurations")
            .and_then(Value::as_array);
        for slot in
            1..=media_slot_count(params.get("referenceVideo"), params.get("referenceVideos"))
        {
            keyframe.insert(format!("hasReferenceVideo{slot}"), json!(true));
            if let Some(duration) = durations.and_then(|values| values.get(slot - 1)) {
                keyframe.insert(
                    format!("referenceVideo{slot}DurationSeconds"),
                    duration.clone(),
                );
            }
        }
    } else {
        for (field, target) in [
            ("referenceAudio", "hasReferenceAudio"),
            ("referenceVideo", "hasReferenceVideo"),
        ] {
            if truthy(params.get(field)) {
                keyframe.insert(target.into(), json!(true));
            }
        }
    }
    if truthy(params.get("referenceAudioIdentity")) {
        keyframe.insert("hasReferenceAudioIdentity".into(), json!(true));
    }
    if truthy(params.get("referenceMask"))
        && params
            .get("controlNet")
            .and_then(|control| control.get("name"))
            .and_then(Value::as_str)
            == Some("inpaint")
    {
        keyframe.insert("hasReferenceMask".into(), json!(true));
    }
    for index in 1_usize..=16 {
        let direct = truthy(params.get(&format!("contextImage{index}")));
        let offset = usize::from(
            is_minimax_h3_reference_model(model_id) && truthy(params.get("referenceImage")),
        );
        let array = index
            .checked_sub(offset + 1)
            .and_then(|array_index| {
                params
                    .get("contextImages")
                    .and_then(Value::as_array)
                    .and_then(|values| values.get(array_index))
            })
            .is_some_and(truthy_value);
        if direct || array {
            keyframe.insert(format!("hasContextImage{index}"), json!(true));
        }
    }
    for (source, target) in [
        ("referenceImageUrls", "referenceImageURLs"),
        ("referenceAudioUrls", "referenceAudioURLs"),
        ("referenceVideoUrls", "referenceVideoURLs"),
        ("referenceFileUrl", "referenceFileURL"),
        ("referenceLinkUrl", "referenceLinkURL"),
        ("promptExtend", "promptExtend"),
        ("watermark", "watermark"),
        ("ratio", "ratio"),
        ("seedanceTaskType", "seedanceTaskType"),
        ("generateAudio", "generateAudio"),
        ("audioIdentityStrength", "identityGuidanceScale"),
        ("frames", "frames"),
        ("shift", "shift"),
        ("audioStart", "audioStart"),
        ("audioDuration", "audioDuration"),
        ("videoStart", "videoStart"),
        ("firstFrameStrength", "firstFrameStrength"),
        ("lastFrameStrength", "lastFrameStrength"),
        ("detailerStrength", "detailerStrength"),
        ("outpaintPosition", "outpaintPosition"),
    ] {
        copy_if_present(params, keyframe, source, target);
    }
    if let Some(value) = params
        .get("teacacheThreshold")
        .filter(|value| !value.is_null())
    {
        keyframe.insert(
            "teacacheThreshold".into(),
            json!(ranged_number(Some(value), "teacacheThreshold", 0.0, 1.0)?),
        );
    }
    let explicit_fps = params.get("fps").filter(|value| !value.is_null());
    let fps = if let Some(value) = explicit_fps {
        let fps = number(Some(value))
            .filter(|value| *value > 0.0)
            .ok_or_else(|| Error::InvalidInput("fps must be a positive finite number".into()))?;
        keyframe.insert("fps".into(), json!(fps));
        fps
    } else if is_wan3_model(model_id) {
        keyframe.insert("fps".into(), json!(30));
        30.0
    } else if is_external_video_model(model_id) || is_minimax_h3_model(model_id) {
        keyframe.insert("fps".into(), json!(24));
        24.0
    } else {
        24.0
    };
    if let Some(value) = params.get("duration").filter(|value| !value.is_null()) {
        let duration = number(Some(value))
            .ok_or_else(|| Error::InvalidInput("video duration must be a finite number".into()))?;
        let minimum = if is_minimax_h3_model(model_id) {
            MINIMAX_H3_MIN_DURATION
        } else if is_wan3_model(model_id) {
            2.0
        } else if is_happyhorse_model(model_id) {
            3.0
        } else if is_seedance_model(model_id) {
            4.0
        } else {
            1.0
        };
        let maximum = if is_minimax_h3_model(model_id) {
            MINIMAX_H3_MAX_DURATION
        } else if is_seedance25_model(model_id) || is_wan3_model(model_id) {
            30.0
        } else if is_external_video_model(model_id) {
            15.0
        } else if is_ltx_model(model_id) || model_id.contains("_animate-") {
            20.0
        } else {
            10.0
        };
        if duration < minimum || duration > maximum {
            return Err(Error::InvalidInput(format!(
                "video duration must be between {minimum} and {maximum}"
            )));
        }
        let frames = calculate_video_frames(model_id, duration, fps, None, None)?;
        keyframe.insert("frames".into(), json!(frames));
    }
    if let Some(coordinates) = params.get("sam2Coordinates") {
        keyframe.insert(
            "sam2Coordinates".into(),
            Value::String(serde_json::to_string(coordinates)?),
        );
    }
    if truthy(params.get("trimEndFrame")) {
        keyframe.insert("trimEndFrame".into(), json!(true));
    }
    if let (Some(width), Some(height)) = (params.get("width"), params.get("height")) {
        let minimum = if is_minimax_h3_model(model_id) {
            32.0
        } else {
            480.0
        };
        let width = ranged_number(Some(width), "video width", minimum, 15_360.0)?;
        let height = ranged_number(Some(height), "video height", minimum, 15_360.0)?;
        keyframe.insert("width".into(), json!(width));
        keyframe.insert("height".into(), json!(height));
    }
    if let Some(control) = params.get("controlNet").and_then(Value::as_object) {
        let name = control
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("controlNet.name is required".into()))?;
        let mut raw = json!({"name": name});
        if let Some(strength) = control.get("strength") {
            raw["controlStrength"] = json!(ranged_number(
                Some(strength),
                "controlNet.strength",
                0.0,
                1.0,
            )?);
        }
        keyframe.insert("currentControlNetsJob".into(), json!([raw]));
    }
    if let Some(value) =
        validate_option(params.get("sampler"), options.raw.get("sampler"), "sampler")?
    {
        keyframe.insert("comfySampler".into(), value);
    } else {
        keyframe.insert("comfySampler".into(), Value::Null);
    }
    if let Some(value) = validate_option(
        params.get("scheduler"),
        options.raw.get("scheduler"),
        "scheduler",
    )? {
        keyframe.insert("comfyScheduler".into(), value);
    } else {
        keyframe.insert("comfyScheduler".into(), Value::Null);
    }
    Ok(())
}
