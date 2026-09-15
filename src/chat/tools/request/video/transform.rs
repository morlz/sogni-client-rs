use super::*;

pub(in crate::chat::tools::request) fn transform(
    args: &Value,
    options: &Value,
    models: &[Value],
) -> Result<ToolRequestPlan> {
    let input = string(args, "reference_video_url")
        .ok_or_else(|| Error::InvalidInput("video_to_video requires reference_video_url".into()))?;
    let control = string(args, "control_mode")
        .filter(|value| {
            matches!(
                *value,
                "animate-replace"
                    | "seedance-v2v"
                    | "canny"
                    | "pose"
                    | "depth"
                    | "detailer"
                    | "outpaint"
                    | "inpaint"
            )
        })
        .unwrap_or("animate-move");
    let animate = control == "animate-move" || control == "animate-replace";
    let workflows = [if animate { control } else { "v2v" }];
    let preferred: &[&str] = match control {
        "animate-move" => &["wan_v2.2-14b-fp8_animate-move_lightx2v"],
        "animate-replace" => &["wan_v2.2-14b-fp8_animate-replace_lightx2v"],
        "seedance-v2v" => &["seedance-2-0", "ltx23-22b-fp8_v2v_distilled"],
        _ => &["ltx23-22b-fp8_v2v_distilled"],
    };
    let requested = resolve_hosted_tool_model_selector("video_to_video", args);
    let model = select(
        models,
        "video",
        requested.as_deref(),
        Some(&workflows),
        preferred,
        None,
    )?;
    let external = is_external_video_model(&model);
    let mut plan = ToolRequestPlan::new("video", &model, args, options)?;
    if animate && string(args, "reference_image_url").is_none() {
        return Err(Error::InvalidInput(format!(
            "{control} requires reference_image_url"
        )));
    }
    plan.dimensions(args, &model, false);
    plan.params.insert(
        "duration".into(),
        json!(args.get("duration").and_then(Value::as_f64).unwrap_or(5.0)),
    );
    plan.asset(AssetRole::ReferenceVideo, input, "video", false);
    plan.copy(
        args,
        &[
            ("outputFormat", "outputFormat"),
            ("returnLastFrame", "returnLastFrame"),
            ("seed", "seed"),
            ("audio_identity_strength", "audioIdentityStrength"),
            ("video_start", "videoStart"),
        ],
    );
    if !external {
        plan.copy(
            args,
            &[
                ("negativePrompt", "negativePrompt"),
                ("detailer_strength", "detailerStrength"),
            ],
        );
    }
    if let Some(image) = string(args, "reference_image_url") {
        plan.asset(AssetRole::ReferenceImage, image, "image", false);
    }
    if let Some(audio) = string(args, "reference_audio_identity_url") {
        plan.asset(AssetRole::ReferenceAudioIdentity, audio, "audio", false);
    }
    if let Some(value) = args.get("generateAudio").and_then(Value::as_bool) {
        plan.params.insert("generateAudio".into(), json!(value));
    }
    if !animate && !external {
        plan.params.insert(
            "controlNet".into(),
            json!({"name":control,"strength":if control == "detailer" { 1.0 } else { 0.85 }}),
        );
    }
    Ok(plan)
}
