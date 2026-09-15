use super::*;
pub(in crate::projects) fn validate_h3_params(
    params: &Map<String, Value>,
    model_id: &str,
) -> Result<()> {
    if !is_minimax_h3_model(model_id) {
        return Ok(());
    }
    if let Some(fps) = params.get("fps").filter(|value| !value.is_null()) {
        if fps.as_f64() != Some(24.0) {
            return Err(Error::InvalidInput(
                "MiniMax H3 fps is fixed at 24. Omit fps or set it to 24".into(),
            ));
        }
    }
    let (expected_steps, tier) = if is_minimax_h3_turbo_model(model_id) {
        (4.0, " Turbo")
    } else if is_minimax_h3_balanced_model(model_id) {
        (8.0, " Balanced")
    } else {
        (20.0, "")
    };
    if let Some(steps) = params.get("steps").filter(|value| !value.is_null()) {
        if steps.as_f64() != Some(expected_steps) {
            return Err(Error::InvalidInput(format!(
                "MiniMax H3{tier} steps are fixed at {}",
                expected_steps as u8
            )));
        }
    }
    if let Some(guidance) = params.get("guidance").filter(|value| !value.is_null()) {
        if guidance.as_f64() != Some(1.0) {
            return Err(Error::InvalidInput(
                "MiniMax H3 guidance is fixed at 1".into(),
            ));
        }
    }
    if params.get("negativePrompt").is_some_and(|value| {
        value
            .as_str()
            .map_or_else(|| truthy_value(value), |text| !text.trim().is_empty())
    }) {
        return Err(Error::InvalidInput(
            "MiniMax H3 has no negative-prompt input. Put requested exclusions in positivePrompt"
                .into(),
        ));
    }
    if let Some(value) = params.get("frames").filter(|value| !value.is_null()) {
        let frames = value.as_i64().ok_or_else(|| {
            Error::InvalidInput(
                "MiniMax H3 frames must be 124 + n*17 in the inclusive range 124-362".into(),
            )
        })?;
        if !(MINIMAX_H3_MIN_FRAMES..=MINIMAX_H3_MAX_FRAMES).contains(&frames)
            || (frames - MINIMAX_H3_BASE_FRAMES) % MINIMAX_H3_FRAME_STEP != 0
        {
            return Err(Error::InvalidInput(
                "MiniMax H3 frames must be 124 + n*17 in the inclusive range 124-362".into(),
            ));
        }
    }
    let width = params.get("width").filter(|value| !value.is_null());
    let height = params.get("height").filter(|value| !value.is_null());
    if width.is_some() != height.is_some() {
        return Err(Error::InvalidInput(
            "MiniMax H3 width and height must be provided together".into(),
        ));
    }
    if let (Some(width), Some(height)) = (width, height) {
        let dimensions = width.as_i64().zip(height.as_i64());
        let valid = dimensions.is_some_and(|(width, height)| {
            width >= MINIMAX_H3_DIMENSION_STEP
                && height >= MINIMAX_H3_DIMENSION_STEP
                && width <= MINIMAX_H3_MAX_DIMENSION
                && height <= MINIMAX_H3_MAX_DIMENSION
                && width % MINIMAX_H3_DIMENSION_STEP == 0
                && height % MINIMAX_H3_DIMENSION_STEP == 0
                && width * height <= MINIMAX_H3_MAX_PIXELS
        });
        if !valid {
            return Err(Error::InvalidInput(
                "MiniMax H3 dimensions must use a 32px grid, stay at or below 1344px per axis, and fit within 1,032,192 pixels"
                    .into(),
            ));
        }
    }
    // The worker derives the uploaded-audio window from frames/24.
    if params.contains_key("audioDuration") {
        return Err(Error::InvalidInput("MiniMax H3 has no audioDuration input. Set frames or duration; the uploaded audio is trimmed to the video length.".into()));
    }
    if is_minimax_h3_audio_guide_model(model_id) {
        let workflow = get_video_workflow_type(model_id).unwrap_or("audio-guide");
        if params.get("generateAudio") == Some(&json!(false)) {
            return Err(Error::InvalidInput(format!(
                "MiniMax H3 {workflow} output always carries the uploaded audio. Omit generateAudio or set it to true."
            )));
        }
        if let Some(value) = params.get("audioStart") {
            if !value
                .as_f64()
                .is_some_and(|value| value.is_finite() && value >= 0.0)
            {
                return Err(Error::InvalidInput(format!(
                    "MiniMax H3 {workflow} audioStart must be a number of seconds, 0 or greater."
                )));
            }
        }
        if ["loras", "loraStrengths"].iter().any(|field| {
            params
                .get(*field)
                .is_some_and(|value| !value.as_array().is_some_and(Vec::is_empty))
        }) {
            return Err(Error::InvalidInput(format!(
                "MiniMax H3 {workflow} does not support LoRAs. Remove loras and loraStrengths."
            )));
        }
    } else if params.contains_key("audioStart") {
        return Err(Error::InvalidInput("audioStart is supported only by the MiniMax H3 FastH3 audio-guide workflows (minimax-h3-fastvideo-int8_ia2v_turbo, minimax-h3-fastvideo-int8_flfa2v_turbo, minimax-h3-fastvideo-int8_a2v_turbo and their _2stage ids).".into()));
    }
    Ok(())
}

pub(super) fn validate_h3_references(params: &Map<String, Value>) -> Result<()> {
    for field in [
        "referenceImageUrls",
        "referenceVideoUrls",
        "referenceAudioUrls",
    ] {
        if params.get(field).is_some_and(|value| !value.is_null()) {
            return Err(Error::InvalidInput(format!(
                "MiniMax H3 r2v does not accept {field}; pass files through the Sogni asset upload fields instead"
            )));
        }
    }
    let images = usize::from(truthy(params.get("referenceImage")))
        + params
            .get("contextImages")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
        + direct_context_image_count(params);
    let videos = media_slot_count(params.get("referenceVideo"), params.get("referenceVideos"));
    let audios = media_slot_count(params.get("referenceAudio"), params.get("referenceAudios"));
    if let Some(value) = params
        .get("referenceVideoDurations")
        .filter(|value| !value.is_null())
    {
        let durations = value.as_array().ok_or_else(|| {
            Error::InvalidInput(format!(
                "MiniMax H3 r2v referenceVideoDurations must contain one entry for each uploaded reference video (expected {videos})"
            ))
        })?;
        if durations.len() != videos {
            return Err(Error::InvalidInput(format!(
                "MiniMax H3 r2v referenceVideoDurations must contain one entry for each uploaded reference video (expected {videos})"
            )));
        }
        let mut total = 0.0;
        for (index, value) in durations.iter().enumerate() {
            let Some(duration) = value.as_f64().filter(|duration| duration.is_finite()) else {
                return Err(Error::InvalidInput(format!(
                    "MiniMax H3 r2v referenceVideoDurations[{index}] must be between 2 and 15 seconds"
                )));
            };
            if !(1.95..=15.05).contains(&duration) {
                return Err(Error::InvalidInput(format!(
                    "MiniMax H3 r2v referenceVideoDurations[{index}] must be between 2 and 15 seconds"
                )));
            }
            total += duration;
        }
        if total > 15.05 {
            return Err(Error::InvalidInput(format!(
                "MiniMax H3 r2v reference videos may total at most 15 seconds (got {total})"
            )));
        }
    }
    for (count, maximum, label) in [
        (images, MINIMAX_H3_MAX_REFERENCE_IMAGES, "reference images"),
        (videos, MINIMAX_H3_MAX_REFERENCE_VIDEOS, "reference videos"),
        (audios, MINIMAX_H3_MAX_REFERENCE_AUDIOS, "reference audios"),
    ] {
        if count > maximum {
            return Err(Error::InvalidInput(format!(
                "MiniMax H3 r2v supports at most {maximum} uploaded {label} (got {count})"
            )));
        }
    }
    let total = images + videos + audios;
    if total > MINIMAX_H3_MAX_REFERENCE_FILES {
        return Err(Error::InvalidInput(format!(
            "MiniMax H3 r2v supports at most 12 reference files in total (got {total}: {images} image, {videos} video, {audios} audio)"
        )));
    }
    if images + videos == 0 {
        return Err(Error::InvalidInput(
            "MiniMax H3 r2v needs at least one uploaded visual reference".into(),
        ));
    }
    Ok(())
}
