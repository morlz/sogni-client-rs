use super::*;
mod assets;
mod external;
mod gpt_image;
mod h3;
pub(super) use assets::{custom_image_size_bounds, video_asset_requirements};
use external::{seedance_reference_limits, validate_seedance_task, validate_wan3_references};
pub(super) use gpt_image::validate_gpt_image_options;
pub(super) use h3::validate_h3_params;
use h3::validate_h3_references;

pub(in crate::projects) const RETIRED_OUTPUT_SCALE_MESSAGE: &str = "outputScale is no longer supported. For MiniMax H3 1080p or 2K output use the two-stage model ids minimax-h3-fastvideo-int8_t2v_turbo_2stage, minimax-h3-fastvideo-int8_i2v_turbo_2stage or minimax-h3-fastvideo-int8_flf2v_turbo_2stage.";

pub(in crate::projects) fn reject_retired_output_scale(params: &Map<String, Value>) -> Result<()> {
    if params.contains_key("outputScale") {
        return Err(Error::InvalidInput(RETIRED_OUTPUT_SCALE_MESSAGE.into()));
    }
    Ok(())
}
pub(super) fn validate_project_params(params: &Map<String, Value>) -> Result<()> {
    for field in ["type", "modelId", "positivePrompt"] {
        required_str(params, field)?;
    }
    let count = params
        .get("numberOfMedia")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::InvalidInput("numberOfMedia must be a positive integer".into()))?;
    if count == 0 || count > u64::from(u32::MAX) {
        return Err(Error::InvalidInput(
            "numberOfMedia must be a positive integer".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_video_assets(params: &Map<String, Value>, model_id: &str) -> Result<()> {
    if let Some(format) = params.get("outputFormat") {
        if !matches!(format.as_str(), Some("mp4" | "mov")) {
            return Err(Error::InvalidInput(
                "Video outputFormat must be mp4 or mov.".into(),
            ));
        }
        if format == "mov" && !is_seedance25_model(model_id) {
            return Err(Error::InvalidInput(
                "MOV output is supported only by Seedance 2.5.".into(),
            ));
        }
    }
    if let Some(value) = params.get("returnLastFrame") {
        let value = value
            .as_bool()
            .ok_or_else(|| Error::InvalidInput("returnLastFrame must be a boolean.".into()))?;
        if value && !is_seedance25_model(model_id) {
            return Err(Error::InvalidInput(
                "Last-frame export is supported only by Seedance 2.5.".into(),
            ));
        }
    }
    for field in ["contextImages", "referenceVideos", "referenceAudios"] {
        if let Some(value) = params.get(field).filter(|value| !value.is_null()) {
            let values = value.as_array().ok_or_else(|| {
                Error::InvalidInput(format!("{field} must be an array without empty entries"))
            })?;
            if values.iter().any(|value| !truthy_value(value)) {
                return Err(Error::InvalidInput(format!(
                    "{field} must be an array without empty entries"
                )));
            }
            if !is_minimax_h3_reference_model(model_id) {
                return Err(Error::InvalidInput(format!(
                    "{field} is supported only by MiniMax H3 r2v models"
                )));
            }
        }
    }
    if params
        .get("referenceVideoDurations")
        .is_some_and(|value| !value.is_null())
        && !is_minimax_h3_reference_model(model_id)
    {
        return Err(Error::InvalidInput(
            "referenceVideoDurations is supported only by MiniMax H3 r2v models".into(),
        ));
    }

    let image_urls =
        validate_reference_array(params.get("referenceImageUrls"), "referenceImageUrls")?;
    let video_urls =
        validate_reference_array(params.get("referenceVideoUrls"), "referenceVideoUrls")?;
    let audio_urls =
        validate_reference_array(params.get("referenceAudioUrls"), "referenceAudioUrls")?;

    if is_happyhorse_model(model_id) {
        if truthy(params.get("referenceVideo")) || !video_urls.is_empty() {
            return Err(Error::InvalidInput(
                "HappyHorse models do not support reference video assets".into(),
            ));
        }
        if truthy(params.get("referenceAudio"))
            || truthy(params.get("referenceAudioIdentity"))
            || !audio_urls.is_empty()
        {
            return Err(Error::InvalidInput(
                "HappyHorse models do not support reference audio assets".into(),
            ));
        }
        if truthy(params.get("referenceImageEnd")) {
            return Err(Error::InvalidInput(
                "HappyHorse models do not support a separate end-frame image (referenceImageEnd)"
                    .into(),
            ));
        }
        let workflow = get_video_workflow_type(model_id);
        let image_count = usize::from(truthy(params.get("referenceImage"))) + image_urls.len();
        match workflow {
            Some("i2v") if image_count != 1 => {
                return Err(Error::InvalidInput(
                    "HappyHorse i2v requires exactly one first-frame reference image".into(),
                ));
            }
            Some("r2v") if !(1..=9).contains(&image_count) => {
                return Err(Error::InvalidInput(
                    "HappyHorse r2v requires between 1 and 9 reference images".into(),
                ));
            }
            Some("t2v") if image_count != 0 => {
                return Err(Error::InvalidInput(
                    "HappyHorse t2v does not support reference images".into(),
                ));
            }
            _ => {}
        }
        return Ok(());
    }
    if is_wan3_model(model_id) {
        return validate_wan3_references(params, &image_urls, &video_urls, &audio_urls);
    }
    if is_seedance_model(model_id) {
        validate_seedance_task(params, &image_urls, &video_urls, &audio_urls)?;
        let image_count = usize::from(truthy(params.get("referenceImage")))
            + usize::from(truthy(params.get("referenceImageEnd")))
            + image_urls.len();
        let video_count = usize::from(truthy(params.get("referenceVideo"))) + video_urls.len();
        let audio_count = usize::from(
            truthy(params.get("referenceAudio")) || truthy(params.get("referenceAudioIdentity")),
        ) + audio_urls.len();
        let (image_max, video_max, audio_max, total_max) = seedance_reference_limits(model_id)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "unknown Seedance model {model_id}; no reference-asset limits are defined"
                ))
            })?;
        if image_count > image_max {
            return Err(Error::InvalidInput(format!(
                "{model_id} supports at most {image_max} image assets"
            )));
        }
        if video_count > video_max {
            return Err(Error::InvalidInput(format!(
                "{model_id} supports at most {video_max} video assets"
            )));
        }
        if audio_count > audio_max {
            return Err(Error::InvalidInput(format!(
                "{model_id} supports at most {audio_max} audio assets"
            )));
        }
        if image_count + video_count + audio_count > total_max {
            return Err(Error::InvalidInput(format!(
                "{model_id} supports at most {total_max} total asset files"
            )));
        }
        if !is_seedance25_model(model_id)
            && audio_count != 0
            && image_count == 0
            && video_count == 0
        {
            return Err(Error::InvalidInput(
                "Seedance audio references require at least one image or video reference".into(),
            ));
        }
        return Ok(());
    }
    if is_minimax_h3_reference_model(model_id) {
        validate_h3_references(params)?;
    } else if !image_urls.is_empty()
        || !video_urls.is_empty()
        || !audio_urls.is_empty()
        || truthy(params.get("referenceFileUrl"))
        || truthy(params.get("referenceLinkUrl"))
    {
        return Err(Error::InvalidInput(
            "external reference URLs are supported only by Seedance, HappyHorse, and Wan 3 models"
                .into(),
        ));
    }

    let Some(workflow) = get_video_workflow_type(model_id) else {
        return Ok(());
    };
    if workflow == "i2v"
        && !truthy(params.get("referenceImage"))
        && !truthy(params.get("referenceImageEnd"))
    {
        return Err(Error::InvalidInput(
            "i2v workflow requires at least one of referenceImage or referenceImageEnd".into(),
        ));
    }
    if truthy(params.get("sam2Coordinates")) && workflow != "animate-replace" {
        return Err(Error::InvalidInput(
            "sam2Coordinates is only supported for animate-replace workflows".into(),
        ));
    }
    for (asset, requirement) in video_asset_requirements(model_id, workflow) {
        let present = truthy(params.get(&asset));
        if requirement == "required" && !present {
            return Err(Error::InvalidInput(format!(
                "{workflow} workflow requires {asset}. Please provide this asset"
            )));
        }
        if requirement == "forbidden" && present {
            return Err(Error::InvalidInput(format!(
                "{workflow} workflow does not support {asset}. Please remove this asset"
            )));
        }
    }
    Ok(())
}

fn validate_reference_array<'a>(value: Option<&'a Value>, name: &str) -> Result<Vec<&'a str>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        Error::InvalidInput(format!("{name} must contain only non-empty URL strings"))
    })?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    Error::InvalidInput(format!("{name} must contain only non-empty URL strings"))
                })
        })
        .collect()
}

pub(super) fn media_slot_count(single: Option<&Value>, multiple: Option<&Value>) -> usize {
    usize::from(truthy(single))
        + multiple
            .and_then(Value::as_array)
            .map(|values| values.iter().filter(|value| truthy_value(value)).count())
            .unwrap_or(0)
}

fn direct_context_image_count(params: &Map<String, Value>) -> usize {
    (1..=16)
        .filter(|slot| truthy(params.get(&format!("contextImage{slot}"))))
        .count()
}
