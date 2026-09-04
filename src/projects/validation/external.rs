use super::*;
pub(super) fn validate_seedance_task(
    params: &Map<String, Value>,
    image_urls: &[&str],
    video_urls: &[&str],
    audio_urls: &[&str],
) -> Result<()> {
    let model_id = required_str(params, "modelId")?;
    let task = match params
        .get("seedanceTaskType")
        .filter(|value| !value.is_null())
    {
        Some(value) => Some(value.as_str().ok_or_else(|| {
            Error::InvalidInput("seedanceTaskType must be reference, edit, or extend".into())
        })?),
        None => None,
    };
    if task.is_some_and(|task| !matches!(task, "reference" | "edit" | "extend")) {
        return Err(Error::InvalidInput(
            "seedanceTaskType must be reference, edit, or extend".into(),
        ));
    }
    if task.is_some() && !is_seedance25_model(model_id) {
        return Err(Error::InvalidInput(
            "seedanceTaskType is supported only by Seedance 2.5".into(),
        ));
    }
    if !is_seedance25_model(model_id) {
        return Ok(());
    }
    let has_frames =
        truthy(params.get("referenceImage")) || truthy(params.get("referenceImageEnd"));
    let has_video = truthy(params.get("referenceVideo")) || !video_urls.is_empty();
    let has_loose = !image_urls.is_empty()
        || has_video
        || truthy(params.get("referenceAudio"))
        || !audio_urls.is_empty();
    if task.is_none() && has_loose {
        return Err(Error::InvalidInput(
            "Seedance 2.5 loose-reference requests require seedanceTaskType".into(),
        ));
    }
    if task.is_some() && has_frames {
        return Err(Error::InvalidInput(
            "seedanceTaskType is for Seedance 2.5 loose-reference, edit, or extend requests; omit it for first/last-frame generation"
                .into(),
        ));
    }
    if matches!(task, Some("edit" | "extend")) && !has_video {
        return Err(Error::InvalidInput(format!(
            "Seedance 2.5 {} requires at least one reference video",
            task.expect("matched Some above")
        )));
    }
    if task == Some("reference") && !has_loose {
        return Err(Error::InvalidInput(
            "Seedance 2.5 reference requires at least one loose image, video, or audio reference"
                .into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_wan3_references(
    params: &Map<String, Value>,
    image_urls: &[&str],
    video_urls: &[&str],
    audio_urls: &[&str],
) -> Result<()> {
    let model_id = required_str(params, "modelId")?;
    let enhanced = is_wan3_enhanced_model(model_id);
    for field in ["referenceFileUrl", "referenceLinkUrl"] {
        if let Some(value) = params.get(field).filter(|value| !value.is_null()) {
            if !value
                .as_str()
                .is_some_and(|value| value.trim().starts_with("https://"))
            {
                return Err(Error::InvalidInput(format!(
                    "{field} must be a valid public HTTPS URL"
                )));
            }
        }
    }
    let has_file = truthy(params.get("referenceFileUrl"));
    let has_link = truthy(params.get("referenceLinkUrl"));
    if has_file && has_link {
        return Err(Error::InvalidInput(
            "Wan 3 accepts either one reference file or one reference link, not both".into(),
        ));
    }
    if enhanced && (has_file || has_link) {
        return Err(Error::InvalidInput(
            "Wan 3.0 Enhanced does not accept document or webpage references".into(),
        ));
    }
    for field in ["promptExtend", "watermark"] {
        if let Some(value) = params.get(field).filter(|value| !value.is_null()) {
            if !value.is_boolean() {
                return Err(Error::InvalidInput(format!(
                    "Wan 3 {field} must be a boolean"
                )));
            }
        }
    }
    if enhanced
        && params
            .get("watermark")
            .is_some_and(|value| !value.is_null())
    {
        return Err(Error::InvalidInput(
            "Wan 3.0 Enhanced does not expose a watermark option".into(),
        ));
    }
    if params
        .get("smartDuration")
        .is_some_and(|value| !value.is_null())
    {
        return Err(Error::InvalidInput(
            "Wan 3 smartDuration has been retired. Send an explicit duration between 2 and 30 seconds instead"
                .into(),
        ));
    }
    if let Some(fps) = params.get("fps").filter(|value| !value.is_null()) {
        if fps.as_f64() != Some(30.0) {
            return Err(Error::InvalidInput(
                "Wan 3 output is fixed at 30 fps".into(),
            ));
        }
    }
    if let Some(ratio) = params.get("ratio").filter(|value| !value.is_null()) {
        if !ratio.as_str().is_some_and(|ratio| {
            matches!(ratio, "adaptive" | "16:9" | "4:3" | "1:1" | "3:4" | "9:16")
        }) {
            return Err(Error::InvalidInput(
                "Wan 3 ratio must be adaptive, 16:9, 4:3, 1:1, 3:4, or 9:16".into(),
            ));
        }
    }
    if truthy(params.get("referenceAudioIdentity")) || truthy(params.get("referenceMask")) {
        return Err(Error::InvalidInput(
            "Wan 3 does not support audio-identity or mask inputs".into(),
        ));
    }
    if let Some(seed) = params.get("seed").filter(|value| !value.is_null()) {
        if !seed
            .as_i64()
            .is_some_and(|seed| (0..=2_147_483_647).contains(&seed))
        {
            return Err(Error::InvalidInput(
                "Wan 3 seed must be an integer from 0 through 2147483647".into(),
            ));
        }
    }
    let video_count = usize::from(truthy(params.get("referenceVideo"))) + video_urls.len();
    let audio_count = usize::from(truthy(params.get("referenceAudio"))) + audio_urls.len();
    let has_frames =
        truthy(params.get("referenceImage")) || truthy(params.get("referenceImageEnd"));
    let has_document = has_file || has_link;
    let has_loose = !image_urls.is_empty() || video_count != 0 || audio_count != 0 || has_document;
    if !enhanced && truthy(params.get("referenceImageEnd")) && !truthy(params.get("referenceImage"))
    {
        return Err(Error::InvalidInput(
            "Wan 3 last-frame generation requires a first-frame referenceImage".into(),
        ));
    }
    if has_frames && has_loose {
        return Err(Error::InvalidInput(
            "Wan 3 first/last-frame anchors cannot be combined with loose media, file, or link references"
                .into(),
        ));
    }
    if image_urls.len() > 10 {
        return Err(Error::InvalidInput(
            "Wan 3 supports at most 10 reference images".into(),
        ));
    }
    if video_count > 5 {
        return Err(Error::InvalidInput(
            "Wan 3 supports at most 5 reference videos".into(),
        ));
    }
    if audio_count > 5 {
        return Err(Error::InvalidInput(
            "Wan 3 supports at most 5 reference audio clips".into(),
        ));
    }
    let has_prompt = params
        .get("positivePrompt")
        .and_then(Value::as_str)
        .is_some_and(|prompt| !prompt.trim().is_empty());
    if !has_prompt && !has_frames && !has_loose {
        return Err(Error::InvalidInput(
            "Wan 3 requires a prompt or at least one media, file, or link input".into(),
        ));
    }
    Ok(())
}

pub(super) fn seedance_reference_limits(model_id: &str) -> Option<(usize, usize, usize, usize)> {
    match model_id {
        "seedance-2-0" | "seedance-2-0-mini" | "seedance-2-0-fast" => Some((9, 3, 3, 12)),
        "seedance-2-5" => Some((30, 10, 10, 50)),
        _ => None,
    }
}
