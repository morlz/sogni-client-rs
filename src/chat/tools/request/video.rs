use super::*;
use crate::utils::{
    calculate_video_frames, get_video_workflow_type, is_external_video_model, is_minimax_h3_model,
};

mod sound;
mod transform;
pub(super) use sound::sound;
pub(super) use transform::transform;

pub(super) fn generate(args: &Value, options: &Value, models: &[Value]) -> Result<ToolRequestPlan> {
    let images = media_indices(args, options, "referenceImageIndices", "image")?;
    let videos = media_indices(args, options, "referenceVideoIndices", "video")?;
    let audio = media_indices(args, options, "referenceAudioIndices", "audio")?;
    let first = string(args, "reference_image_url");
    let last = string(args, "reference_image_end_url");
    if !images.is_empty() && (first.is_some() || last.is_some()) {
        return Err(Error::InvalidInput("Use either referenceImageIndices with mediaContext or legacy inline reference_image_url arguments, not both".into()));
    }
    let has_images = !images.is_empty() || first.is_some() || last.is_some();
    if has_images
        && string(args, "videoModel").is_some_and(|name| {
            ["minimax-h3-t2v", "minimax-h3-t2v-turbo"]
                .contains(&name.trim().to_lowercase().as_str())
        })
    {
        return Err(Error::InvalidInput(format!(
            "{} does not accept reference images; use the matching MiniMax H3 i2v or flf2v project workflow",
            args["videoModel"].as_str().unwrap().trim().to_lowercase()
        )));
    }
    if args.get("videoModel").and_then(Value::as_str) == Some("minimax-h3-r2v") {
        return Err(Error::InvalidInput("generate_video with videoModel 'minimax-h3-r2v' is not supported by direct SDK tool execution; use hosted chat, durable runs, or projects.create".into()));
    }
    let mut routing_args = args.clone();
    if has_images && images.is_empty() {
        routing_args["referenceImageIndices"] = json!([0]);
    }
    let requested = resolve_hosted_tool_model_selector("generate_video", &routing_args);
    let workflow = requested.as_deref().and_then(get_video_workflow_type);
    let (workflows, preferred): (&[&str], &[&str]) = match workflow {
        Some("flf2v") => (&["flf2v"], &["minimax-h3-fl2va-fp8_flf2v"]),
        Some("r2v") => (&["r2v"], &["happyhorse-1.1-r2v"]),
        _ if has_images => (&["i2v"], &["ltx23-22b-fp8_i2v_distilled"]),
        _ => (&["t2v"], &["ltx23-22b-fp8_t2v_distilled"]),
    };
    let model = select(
        models,
        "video",
        requested.as_deref(),
        Some(workflows),
        preferred,
        None,
    )?;
    let external = is_external_video_model(&model);
    let mut plan = ToolRequestPlan::new("video", &model, args, options)?;
    plan.dimensions(args, &model, true);
    plan.copy(
        args,
        &[
            ("outputFormat", "outputFormat"),
            ("returnLastFrame", "returnLastFrame"),
            ("seed", "seed"),
        ],
    );
    if let Some(prompt) = string(args, "negativePrompt") {
        if is_minimax_h3_model(&model) {
            return Err(Error::InvalidInput(
                "MiniMax H3 has no negative-prompt input; put exclusions in prompt".into(),
            ));
        }
        if !external {
            plan.params.insert("negativePrompt".into(), json!(prompt));
        }
    }
    if let Some(duration) = args.get("duration").and_then(Value::as_f64) {
        if is_minimax_h3_model(&model) {
            plan.params.insert(
                "frames".into(),
                json!(calculate_video_frames(&model, duration, 24.0, None, None)?),
            );
        } else {
            plan.params.insert("duration".into(), json!(duration));
        }
    }
    if let Some(input) = first {
        plan.asset(AssetRole::ReferenceImage, input, "image", false);
    }
    if let Some(input) = last {
        plan.asset(AssetRole::ReferenceImageEnd, input, "image", false);
    }
    if !images.is_empty() {
        if external {
            external_references(
                &mut plan,
                &images,
                "image",
                AssetRole::ReferenceImage,
                "referenceImageUrls",
            )?;
        } else {
            let max = if workflow == Some("flf2v") { 2 } else { 1 };
            if images.len() > max {
                return Err(Error::InvalidInput(format!(
                    "{} accepts at most {max} image input(s)",
                    workflow.unwrap_or("video")
                )));
            }
            plan.asset(AssetRole::ReferenceImage, &images[0], "image", true);
            if let Some(input) = images.get(1) {
                plan.asset(AssetRole::ReferenceImageEnd, input, "image", true);
            }
        }
    }
    if !videos.is_empty() || !audio.is_empty() {
        if !external {
            return Err(Error::InvalidInput(
                "Loose referenceVideoIndices/referenceAudioIndices require an external video model"
                    .into(),
            ));
        }
        external_references(
            &mut plan,
            &videos,
            "video",
            AssetRole::ReferenceVideo,
            "referenceVideoUrls",
        )?;
        external_references(
            &mut plan,
            &audio,
            "audio",
            AssetRole::ReferenceAudio,
            "referenceAudioUrls",
        )?;
    }
    if let Some(input) = string(args, "reference_audio_identity_url") {
        plan.asset(AssetRole::ReferenceAudioIdentity, input, "audio", false);
    }
    plan.copy(
        args,
        &[
            ("audio_identity_strength", "audioIdentityStrength"),
            ("first_frame_strength", "firstFrameStrength"),
            ("last_frame_strength", "lastFrameStrength"),
        ],
    );
    if let Some(value) = args.get("generateAudio").and_then(Value::as_bool) {
        plan.params.insert("generateAudio".into(), json!(value));
    }
    Ok(plan)
}

fn media_indices(args: &Value, options: &Value, field: &str, media: &str) -> Result<Vec<String>> {
    let Some(value) = args.get(field) else {
        return Ok(Vec::new());
    };
    let indices = value
        .as_array()
        .filter(|array| array.iter().all(|entry| entry.as_i64().is_some()))
        .ok_or_else(|| {
            Error::InvalidInput(format!("{field} must contain only integer media indices"))
        })?;
    if indices.is_empty() {
        return Ok(Vec::new());
    }
    let context = options.get("mediaContext").ok_or_else(|| Error::InvalidInput(format!("Indexed {media} arguments require ToolExecutionOptions.mediaContext when using chat.tools.execute()")))?;
    let (generated, uploaded) = match media {
        "image" => ("images", "uploadedImages"),
        "video" => ("videos", "uploadedVideos"),
        _ => ("audio", "uploadedAudio"),
    };
    indices.iter().map(|index| {
        let index = index.as_i64().unwrap();
        let (field, position) = if index >= 0 { (generated, index as u64) } else { (uploaded, index.unsigned_abs() - 1) };
        usize::try_from(position).ok().and_then(|position| context.get(field)?.get(position)?.as_str())
            .filter(|value| !value.trim().is_empty()).map(ToOwned::to_owned)
            .ok_or_else(|| Error::InvalidInput(format!("{media} media index {index} is unavailable in ToolExecutionOptions.mediaContext")))
    }).collect()
}

fn external_references(
    plan: &mut ToolRequestPlan,
    inputs: &[String],
    media: &'static str,
    role: AssetRole,
    field: &str,
) -> Result<()> {
    let (remote, local): (Vec<_>, Vec<_>) = inputs
        .iter()
        .partition(|input| url::Url::parse(input).is_ok_and(|url| url.scheme() == "https"));
    if local.len() > 1 {
        return Err(Error::InvalidInput(format!(
            "Direct external-API video execution supports at most one inline {media}; use HTTPS references for additional {media} inputs"
        )));
    }
    if let Some(input) = local.first() {
        plan.asset(role, input, media, true);
    }
    if !remote.is_empty() {
        plan.params.insert(field.into(), json!(remote));
    }
    Ok(())
}
