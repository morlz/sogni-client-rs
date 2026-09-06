use super::*;
mod image;
mod video;
use image::build_image_keyframe;
use video::build_video_keyframe;

#[cfg(test)]
mod tests;
pub(super) fn build_job_request(
    project_id: &str,
    params: &Map<String, Value>,
    options: &ModelOptions,
    attribution: Option<&WorkloadAttribution>,
) -> Result<Value> {
    let media_type = required_str(params, "type")?;
    if !matches!(media_type, "image" | "video" | "audio") {
        return Err(Error::InvalidInput(
            "project type must be image, video, or audio".into(),
        ));
    }
    if options.media_type != media_type {
        return Err(Error::InvalidInput(format!(
            "model {} does not support {media_type} generation",
            options.model_id
        )));
    }
    let model_id = required_str(params, "modelId")?;
    let mut template = request_template();
    let keyframe = template
        .pointer_mut("/keyFrames/0")
        .and_then(Value::as_object_mut)
        .expect("static request template has a keyframe");
    keyframe.insert("modelID".into(), json!(model_id));
    keyframe.insert(
        "positivePrompt".into(),
        params.get("positivePrompt").cloned().unwrap_or(json!("")),
    );
    copy_if_present(params, keyframe, "steps", "steps");
    copy_if_present(params, keyframe, "guidance", "guidanceScale");
    copy_if_present(params, keyframe, "seed", "seed");
    copy_if_present(params, keyframe, "stylePrompt", "stylePrompt");
    copy_if_present(params, keyframe, "loras", "loras");
    copy_if_present(params, keyframe, "loraStrengths", "loraStrengths");
    if let Some(prompt) = params
        .get("negativePrompt")
        .and_then(Value::as_str)
        .filter(|prompt| !prompt.is_empty())
    {
        if media_type == "image"
            || media_type == "video"
                && !(is_external_video_model(model_id) || is_minimax_h3_model(model_id))
        {
            keyframe.insert("negativePrompt".into(), json!(prompt));
        }
    } else if media_type != "image" {
        keyframe.remove("negativePrompt");
    }

    match media_type {
        "image" => build_image_keyframe(params, options, keyframe)?,
        "video" => build_video_keyframe(params, options, keyframe)?,
        "audio" => build_audio_keyframe(params, options, keyframe)?,
        _ => unreachable!(),
    }

    let object = template
        .as_object_mut()
        .expect("static request template is an object");
    object.insert(
        "previews".into(),
        if media_type == "image" {
            params.get("numberOfPreviews").cloned().unwrap_or(json!(0))
        } else {
            json!(0)
        },
    );
    object.insert(
        "numberOfImages".into(),
        params.get("numberOfMedia").cloned().unwrap_or(json!(1)),
    );
    object.insert("jobID".into(), json!(project_id));
    object.insert(
        "disableSafety".into(),
        json!(
            params
                .get("disableNSFWFilter")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    );
    object.insert(
        "outputFormat".into(),
        params.get("outputFormat").cloned().unwrap_or_else(|| {
            json!(match media_type {
                "video" => "mp4",
                "audio" => "mp3",
                _ => "png",
            })
        }),
    );
    for field in ["tokenType", "billingMode", "network", "appSource"] {
        if let Some(value) = params.get(field).filter(|value| !value.is_null()) {
            object.insert(field.into(), value.clone());
        }
    }
    if let Some(attribution) = attribution {
        for (key, value) in attribution.wire_fields() {
            object.insert(key, json!(value));
        }
    }
    // These nulls are worker reset fields, not absent optional parameters.
    // Workers require these fields to interpret the complete default template.
    Ok(template)
}

fn build_audio_keyframe(
    params: &Map<String, Value>,
    options: &ModelOptions,
    keyframe: &mut Map<String, Value>,
) -> Result<()> {
    for field in [
        "duration",
        "bpm",
        "timesignature",
        "language",
        "lyrics",
        "keyscale",
        "composerMode",
        "promptStrength",
        "creativity",
        "shift",
    ] {
        copy_if_present(params, keyframe, field, field);
    }
    if let Some(value) =
        validate_option(params.get("sampler"), options.raw.get("sampler"), "sampler")?
    {
        keyframe.insert("comfySampler".into(), value);
    }
    if let Some(value) = validate_option(
        params.get("scheduler"),
        options.raw.get("scheduler"),
        "scheduler",
    )? {
        keyframe.insert("comfyScheduler".into(), value);
    }
    Ok(())
}

fn request_template() -> Value {
    json!({
        "selectedUpscalingModel": "OFF",
        "cnVideoFramesSketch": [],
        "cnVideoFramesSegmentedSubject": [],
        "cnVideoFramesFace": [],
        "doCanvasBlending": false,
        "animationIsOn": false,
        "cnVideoFramesBoth": [],
        "cnVideoFramesDepth": [],
        "keyFrames": [{
            "stepsIsEnabled": true,
            "siRotation": 0,
            "siDragOffsetIsEnabled": true,
            "strength": 0.5,
            "siZoomScaleIsEnabled": true,
            "isEnabled": true,
            "processing": "CPU, GPU",
            "useLastImageAsGuideImageInAnimation": true,
            "guidanceScaleIsEnabled": true,
            "siImageBackgroundColor": "black",
            "cnDragOffset": [0, 0],
            "scheduler": null,
            "timeStepSpacing": null,
            "steps": 20,
            "cnRotation": 0,
            "guidanceScale": 7.5,
            "siZoomScale": 1,
            "modelID": "",
            "cnRotationIsEnabled": true,
            "negativePrompt": "",
            "startingImageZoomPanIsOn": false,
            "siRotationIsEnabled": true,
            "cnImageBackgroundColor": "clear",
            "strengthIsEnabled": true,
            "siDragOffset": [0, 0],
            "useLastImageAsCNImageInAnimation": false,
            "positivePrompt": "",
            "controlNetZoomPanIsOn": false,
            "cnZoomScaleIsEnabled": true,
            "currentControlNets": null,
            "stylePrompt": "",
            "cnDragOffsetIsEnabled": true,
            "frameIndex": 0,
            "startingImage": null,
            "cnZoomScale": 1
        }],
        "previews": 0,
        "frameRate": 24,
        "generatedVideoSeconds": 10,
        "canvasIsOn": false,
        "cnVideoFrames": [],
        "disableSafety": false,
        "cnVideoFramesSegmentedBackground": [],
        "cnVideoFramesSegmented": [],
        "numberOfImages": 1,
        "cnVideoFramesPose": [],
        "jobID": "",
        "siVideoFrames": []
    })
}
