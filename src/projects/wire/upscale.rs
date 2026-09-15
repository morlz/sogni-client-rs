use super::*;

const TIMING_ERROR: &str =
    "Omit the source timing, or supply the source video’s exact frame count and frame rate.";

pub(super) fn upscale_resolution(params: &Map<String, Value>) -> Result<f64> {
    let resolution = params
        .get("upscaleResolution")
        .filter(|value| !value.is_null())
        .and_then(Value::as_f64)
        .or_else(|| {
            if params
                .get("upscaleResolution")
                .is_some_and(|value| !value.is_null())
            {
                return None;
            }
            number(params.get("width"))
                .zip(number(params.get("height")))
                .map(|(width, height)| width.min(height))
        });
    resolution
        .filter(|value| [1080.0, 1440.0].contains(value))
        .ok_or_else(|| Error::InvalidInput("Choose 1080p or 1440p for video upscaling.".into()))
}

pub(super) fn validate_upscale_params(params: &Map<String, Value>) -> Result<()> {
    for (field, allowed, message) in [
        (
            "detailPreference",
            ["stable", "sharper"],
            "FlashVSR detailPreference must be stable or sharper.",
        ),
        (
            "processingSpeed",
            ["stable", "faster"],
            "FlashVSR processingSpeed must be stable or faster.",
        ),
    ] {
        if let Some(value) = params.get(field).filter(|value| !value.is_null()) {
            if !value.as_str().is_some_and(|value| allowed.contains(&value)) {
                return Err(Error::InvalidInput(message.into()));
            }
        }
    }
    if let Some(seed) = params.get("seed").filter(|value| !value.is_null()) {
        if !seed
            .as_i64()
            .is_some_and(|seed| (-1..=4_294_967_295).contains(&seed))
        {
            return Err(Error::InvalidInput(
                "FlashVSR seed must be -1 (random) or an integer from 0 through 4294967295.".into(),
            ));
        }
    }
    upscale_resolution(params)?;
    if !truthy(params.get("referenceVideo")) {
        return Err(Error::InvalidInput(
            "FlashVSR requires an uploaded referenceVideo.".into(),
        ));
    }
    validate_source_timing(params)?;
    if ["positivePrompt", "negativePrompt"].iter().any(|field| {
        params
            .get(*field)
            .and_then(Value::as_str)
            .is_some_and(|prompt| !prompt.trim().is_empty())
    }) {
        return Err(Error::InvalidInput("FlashVSR is promptless.".into()));
    }
    if ["teacacheThreshold", "videoStart"]
        .iter()
        .any(|field| params.get(*field).is_some_and(|value| !value.is_null()))
        || [
            "trimEndFrame",
            "controlNet",
            "referenceFileUrl",
            "referenceLinkUrl",
            "referenceVideoUrls",
            "referenceImageUrls",
            "referenceAudioUrls",
        ]
        .iter()
        .any(|field| truthy(params.get(*field)))
        || params.get("generateAudio") == Some(&json!(false))
    {
        return Err(Error::InvalidInput("Video upscaling preserves the complete source video and its audio; generation controls are unsupported.".into()));
    }
    if params.get("numberOfMedia").and_then(Value::as_f64) != Some(1.0) {
        return Err(Error::InvalidInput(
            "Upscale one source video per project.".into(),
        ));
    }
    Ok(())
}

fn validate_source_timing(params: &Map<String, Value>) -> Result<()> {
    let invalid = || Error::InvalidInput(TIMING_ERROR.into());
    let fps = params
        .get("fps")
        .map(|value| {
            number(Some(value))
                .filter(|value| (1.0..=60.0).contains(value))
                .ok_or_else(invalid)
        })
        .transpose()?;
    // No client-side clip-length limit: admission verifies the complete source.
    // A duration alone does not identify frames without the exact source rate.
    let frames = if let Some(value) = params.get("frames") {
        Some(value.as_f64().ok_or_else(invalid)?)
    } else if let Some(value) = params.get("duration") {
        Some((number(Some(value)).ok_or_else(invalid)? * fps.ok_or_else(invalid)? + 0.5).floor())
    } else {
        None
    };
    if frames.is_some_and(|frames| !frames.is_finite() || frames < 1.0 || frames.fract() != 0.0) {
        return Err(invalid());
    }
    Ok(())
}
