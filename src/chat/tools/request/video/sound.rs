use super::*;
use crate::utils::{get_minimax_h3_frames_for_audio_duration, is_minimax_h3_audio_guide_model};

pub(in crate::chat::tools::request) fn sound(
    args: &Value,
    options: &Value,
    models: &[Value],
) -> Result<ToolRequestPlan> {
    let audio = string(args, "reference_audio_url")
        .ok_or_else(|| Error::InvalidInput("sound_to_video requires reference_audio_url".into()))?;
    let first = string(args, "reference_image_url");
    let last = string(args, "reference_image_end_url");
    if last.is_some() && first.is_none() {
        return Err(Error::InvalidInput(
            "sound_to_video reference_image_end_url needs reference_image_url as the first frame"
                .into(),
        ));
    }
    let (workflows, preferred): (&[&str], &[&str]) = if last.is_some() {
        (&["flfa2v"], &["minimax-h3-fastvideo-int8_flfa2v_turbo"])
    } else if first.is_some() {
        (
            &["ia2v", "s2v"],
            &[
                "ltx23-22b-fp8_ia2v_distilled",
                "wan_v2.2-14b-fp8_s2v_lightx2v",
            ],
        )
    } else {
        (&["a2v"], &["ltx23-22b-fp8_a2v_distilled"])
    };
    let requested = resolve_hosted_tool_model_selector("sound_to_video", args);
    if let Some(requested) = requested
        .as_deref()
        .filter(|id| is_minimax_h3_audio_guide_model(id))
    {
        let workflow = get_video_workflow_type(requested).unwrap_or("");
        if !workflows.contains(&workflow) {
            let needs = match workflow {
                "flfa2v" => "reference_image_url and reference_image_end_url",
                "ia2v" => "reference_image_url and no reference_image_end_url",
                _ => "no reference images",
            };
            return Err(Error::InvalidInput(format!(
                "{requested} (MiniMax H3 {workflow}) needs {needs}"
            )));
        }
    }
    let model = select(
        models,
        "video",
        requested.as_deref(),
        Some(workflows),
        preferred,
        None,
    )?;
    let mut plan = ToolRequestPlan::new("video", &model, args, options)?;
    let duration = args.get("duration").and_then(Value::as_f64).unwrap_or(5.0);
    let generate_audio = args.get("generateAudio").and_then(Value::as_bool);
    plan.copy(
        args,
        &[
            ("outputFormat", "outputFormat"),
            ("returnLastFrame", "returnLastFrame"),
            ("seed", "seed"),
        ],
    );
    if is_minimax_h3_audio_guide_model(&model) {
        if generate_audio == Some(false) {
            return Err(Error::InvalidInput(format!(
                "{model} output always carries the uploaded audio; generateAudio: false is not supported"
            )));
        }
        let defaults = get_video_defaults(&model);
        plan.params.extend(json!({"width":defaults.width,"height":defaults.height,"fps":24,"frames":get_minimax_h3_frames_for_audio_duration(duration)?}).as_object().unwrap().clone());
        if let Some(value) = args
            .get("audioStart")
            .filter(|v| !v.is_null())
            .or_else(|| args.get("audio_start"))
        {
            plan.params.insert("audioStart".into(), value.clone());
        }
    } else {
        plan.dimensions(args, &model, false);
        plan.params.insert("duration".into(), json!(duration));
        plan.params.insert("audioDuration".into(), json!(duration));
        plan.copy(args, &[("audio_start", "audioStart")]);
        if let Some(value) = generate_audio {
            plan.params.insert("generateAudio".into(), json!(value));
        }
    }
    plan.asset(AssetRole::ReferenceAudio, audio, "audio", false);
    if let Some(input) = first {
        plan.asset(AssetRole::ReferenceImage, input, "image", false);
    }
    if let Some(input) = last {
        plan.asset(AssetRole::ReferenceImageEnd, input, "image", false);
    }
    Ok(plan)
}
