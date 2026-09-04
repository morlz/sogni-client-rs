mod config;

use crate::common::{
    auth::{close, connect, unique_app_id},
    cli::{confirm_estimate, execution_requested, explain_dry_run},
    files::{download_results, ffprobe, require_file},
    progress::wait_with_progress,
    workflow::{print_request, require_model},
};
use anyhow::{Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use sogni_client::{
    AssetRole, MediaSource, Network, ProjectRequest, calculate_video_frames, is_ltx_model,
    is_wan_model,
};

use config::{Args, Control, spec};

fn parse_coords(value: &str) -> Result<[f64; 2]> {
    let values = value
        .trim_matches(['[', ']'])
        .split(',')
        .map(str::trim)
        .map(str::parse::<f64>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if values.len() != 2 {
        bail!("--sam2-coords must be x,y");
    }
    Ok([values[0], values[1]])
}
fn validate_ltx_fps(model: &str, fps: f64) -> Result<()> {
    if is_ltx_model(model) && (!fps.is_finite() || !(1.0..=60.0).contains(&fps)) {
        bail!("LTX FPS must be finite and between 1 and 60");
    }
    Ok(())
}
fn build(
    args: &Args,
    metadata: Option<crate::common::files::MediaMetadata>,
) -> Result<ProjectRequest> {
    let config = spec(&args.model)?;
    let video = args.video.as_ref().expect("required video");
    if config.needs_image && args.image.is_none() {
        bail!("WAN Animate requires --image and --video");
    }
    if config.control
        && !config.allows_extended
        && matches!(args.control, Control::Outpaint | Control::Inpaint)
    {
        bail!("this LTX 2.5 Dev workflow supports canny, pose, depth, or detailer only");
    }
    if args.control == Control::Inpaint && args.mask.is_none() {
        bail!("inpaint requires --mask");
    }
    if args.control == Control::Outpaint
        && !matches!(
            args.outpaint_position.as_deref(),
            Some("center" | "top" | "bottom" | "left" | "right")
        )
    {
        bail!("outpaint requires --outpaint-position center, top, bottom, left, or right");
    }
    if args.sam2_coords.is_some() && !config.replace {
        bail!("--sam2-coords is supported only by animate-replace");
    }
    if !(0.0..=1.0).contains(&args.strength) {
        bail!("strength must be 0 through 1");
    }
    // Source metadata supplies defaults only. Explicit CLI dimensions, duration,
    // and FPS remain authoritative, then dimensions are aligned to the grid.
    let metadata = metadata.unwrap_or_default();
    let width = args.width.or(metadata.width).unwrap_or(config.width) / config.grid * config.grid;
    let height =
        args.height.or(metadata.height).unwrap_or(config.height) / config.grid * config.grid;
    let fps = args.fps.unwrap_or(config.fps);
    validate_ltx_fps(&config.id, fps)?;
    if is_wan_model(&config.id) && ![16.0, 32.0].contains(&fps) {
        bail!("WAN FPS must be 16 or 32");
    }
    let duration = args.duration.or(metadata.duration_seconds).unwrap_or(4.0);
    // Family-aware frame math preserves WAN interpolation and LTX generation
    // semantics instead of treating output FPS uniformly.
    let frames = args.frames.unwrap_or(calculate_video_frames(
        &config.id,
        duration,
        fps,
        Some(config.min),
        Some(config.max),
    )?);
    let mut request = ProjectRequest::video(config.id, &args.prompt)
        .network(Network::Fast)
        .dimensions(width, height)
        .duration(duration)
        .fps(fps)
        .number_of_media(args.batch)
        .steps(args.steps.unwrap_or(config.steps))
        .guidance(args.guidance.unwrap_or(config.guidance))
        .param("frames", frames)
        .param(
            "sampler",
            args.sampler
                .clone()
                .unwrap_or_else(|| config.sampler.into()),
        )
        .param(
            "scheduler",
            args.scheduler
                .clone()
                .unwrap_or_else(|| config.scheduler.into()),
        )
        .param("safeContentFilter", !args.disable_safe_content_filter)
        .param("tokenType", args.token_type.as_str())
        .param("billingMode", args.billing_mode.as_str())
        .asset(AssetRole::ReferenceVideo, MediaSource::Path(video.clone()));
    if let Some(v) = args.shift.or(config.shift) {
        request = request.param("shift", v);
    }
    if let Some(v) = args.seed {
        request = request.param("seed", v);
    }
    if let Some(v) = args.video_start {
        if v < 0.0 {
            bail!("video start must be non-negative");
        }
        request = request.param("videoStart", v);
    }
    if let Some(path) = &args.image {
        request = request.asset(AssetRole::ReferenceImage, MediaSource::Path(path.clone()));
    }
    if config.control {
        // LTX control data is structured; masks and outpaint anchors are attached
        // only to the control modes that consume them.
        request = request.param(
            "controlNet",
            json!({"name":args.control.as_str(),"strength":args.strength}),
        );
        if let Some(v) = args.detailer_strength {
            request = request.param("detailerStrength", v);
        }
        if let Some(path) = &args.mask {
            request = request.asset(AssetRole::ReferenceMask, MediaSource::Path(path.clone()));
        }
        if let Some(v) = &args.outpaint_position {
            request = request.param("outpaintPosition", v.clone());
        }
    }
    if let Some(v) = &args.sam2_coords {
        request = request.param("sam2Coordinates", json!(parse_coords(v)?));
    }
    if let Some(v) = &args.negative {
        request = request.param("negativePrompt", v.clone());
    }
    if let Some(v) = &args.style {
        request = request.param("stylePrompt", v.clone());
    }
    if let Some(path) = &args.identity_audio {
        if !is_ltx_model(request.params()["modelId"].as_str().unwrap_or_default()) {
            bail!("identity audio is supported only by LTX");
        }
        request = request
            .asset(
                AssetRole::ReferenceAudioIdentity,
                MediaSource::Path(path.clone()),
            )
            .param(
                "audioIdentityStrength",
                args.audio_identity_strength.unwrap_or(1.0),
            );
    }
    Ok(request)
}
fn inputs(args: &Args) -> Result<crate::common::files::MediaMetadata> {
    let video = args.video.as_ref().expect("video");
    require_file(video, "source video")?;
    if let Some(v) = &args.image {
        require_file(v, "reference image")?;
    }
    if let Some(v) = &args.mask {
        require_file(v, "inpaint mask")?;
    }
    if let Some(v) = &args.identity_audio {
        require_file(v, "identity audio")?;
    }
    // Probing improves defaults but is not fatal when callers provide overrides.
    match ffprobe(video) {
        Ok(v) => Ok(v),
        Err(error) => {
            eprintln!(
                "Warning: {error}; use --width/--height/--duration/--fps to override fallbacks."
            );
            Ok(Default::default())
        }
    }
}
fn estimate(p: &Value) -> Value {
    json!({"tokenType":p["tokenType"],"model":p["modelId"],"width":p["width"],"height":p["height"],"frames":p["frames"],"fps":p["fps"],"steps":p["steps"],"numberOfMedia":p["numberOfMedia"],"hasVideoInput":true})
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let execute = execution_requested(args.execute, args.dry_run)?;
    let metadata = if execute { Some(inputs(&args)?) } else { None };
    let request = build(&args, metadata)?;
    if !execute {
        print_request(&request)?;
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("sogni-rust-v2v"), Network::Fast).await?;
    let result = async {
        let id = request.params()["modelId"]
            .as_str()
            .expect("model")
            .to_owned();
        require_model(&client.projects, &id).await?;
        // Quote after metadata resolution so dimensions, duration, and frames
        // match the request that will actually be submitted.
        let quote = client
            .projects
            .estimate_video_cost(&estimate(&request.params()))
            .await?;
        confirm_estimate(&quote, args.yes)?;
        let project = client.projects.create(request).await?;
        println!("Project: {}", project.id());
        let urls = wait_with_progress(&project).await?;
        download_results(&urls, &args.output, "video-to-video", "mp4").await?;
        Result::<()>::Ok(())
    }
    .await;
    result.and(close(&client).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_specific_requirements_are_enforced() {
        let base = ["x", "--video", "v.mp4", "--control-type", "inpaint"];
        assert!(build(&Args::try_parse_from(base).unwrap(), None).is_err());
        let args = Args::try_parse_from([
            "x",
            "--video",
            "v.mp4",
            "--control-type",
            "outpaint",
            "--outpaint-position",
            "right",
        ])
        .unwrap();
        assert_eq!(
            build(&args, None).unwrap().params()["outpaintPosition"],
            "right"
        );
    }
    #[test]
    fn ltx_fps_boundaries_and_dry_run_are_validated() {
        let model = "ltx25-22b-int8_v2v_distilled";
        for fps in [1.0, 60.0] {
            assert!(validate_ltx_fps(model, fps).is_ok());
        }
        for fps in [0.0, 61.0, f64::NAN, f64::INFINITY] {
            assert!(validate_ltx_fps(model, fps).is_err());
        }
        assert!(validate_ltx_fps("wan_v2.2-14b-fp8_animate-move_lightx2v", 16.0).is_ok());

        let args = Args::try_parse_from([
            "x",
            "--video",
            "v.mp4",
            "--model",
            model,
            "--fps",
            "100",
            "--dry-run",
        ])
        .unwrap();
        assert!(
            build(&args, None)
                .unwrap_err()
                .to_string()
                .contains("between 1 and 60")
        );
    }
}
