use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use sogni_client::{
    AssetRole, MediaSource, Network, ProjectRequest, calculate_video_frames,
    get_video_workflow_type, is_ltx_model, is_wan_model,
};

use crate::common::{
    auth::{close, connect, unique_app_id},
    cli::{BillingMode, TokenType, confirm_estimate, execution_requested, explain_dry_run},
    files::{download_results, require_file},
    progress::wait_with_progress,
    workflow::{print_request, require_model},
};

const DEFAULT_PROMPT: &str = "A cheerful auburn-haired puppet holds a red umbrella in sparkling rain, sways to a melody, and sings joyfully while the camera slowly pushes in.";

#[derive(Debug, Parser)]
#[command(about = "Text-to-video with WAN 2.2 or LTX 2.3/2.5")]
struct Args {
    #[arg(default_value = DEFAULT_PROMPT)]
    prompt: String,
    #[arg(long, default_value = "wan_v2.2-14b-fp8_t2v_lightx2v")]
    model: String,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long, default_value_t = 5.0)]
    duration: f64,
    #[arg(long)]
    fps: Option<f64>,
    #[arg(long)]
    frames: Option<i64>,
    #[arg(long, default_value_t = 1)]
    batch: u32,
    #[arg(long)]
    seed: Option<u32>,
    #[arg(long)]
    steps: Option<u32>,
    #[arg(long)]
    guidance: Option<f64>,
    #[arg(long)]
    shift: Option<f64>,
    #[arg(long = "comfy-sampler")]
    sampler: Option<String>,
    #[arg(long = "comfy-scheduler")]
    scheduler: Option<String>,
    #[arg(long)]
    negative: Option<String>,
    #[arg(long)]
    style: Option<String>,
    #[arg(long)]
    identity_audio: Option<PathBuf>,
    #[arg(long, requires = "identity_audio")]
    audio_identity_strength: Option<f64>,
    #[arg(long)]
    disable_safe_content_filter: bool,
    #[arg(long, value_enum, default_value = "spark")]
    token_type: TokenType,
    #[arg(long, alias = "billing", value_enum, default_value = "auto")]
    billing_mode: BillingMode,
    #[arg(long, default_value = "output")]
    output: PathBuf,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

#[derive(Clone)]
struct Spec {
    model: String,
    width: u32,
    height: u32,
    fps: f64,
    steps: u32,
    guidance: f64,
    shift: Option<f64>,
    sampler: &'static str,
    scheduler: &'static str,
    min_frames: i64,
    max_frames: i64,
    dimension_step: u32,
}

fn spec(value: &str) -> Result<Spec> {
    // Resolve aliases first so workflow validation and defaults follow the
    // canonical model family rather than the spelling chosen on the CLI.
    let model = match value {
        "lightx2v" => "wan_v2.2-14b-fp8_t2v_lightx2v",
        "quality" => "wan_v2.2-14b-fp8_t2v",
        other => other,
    };
    if get_video_workflow_type(model) != Some("t2v") {
        bail!("model {model} is not a text-to-video model");
    }
    let ltx = is_ltx_model(model);
    let distilled = model.contains("distilled");
    Ok(Spec {
        model: model.into(),
        width: if ltx { 1920 } else { 640 },
        height: if ltx { 1088 } else { 640 },
        fps: if ltx { 24.0 } else { 16.0 },
        steps: if ltx {
            if distilled { 8 } else { 30 }
        } else if model.contains("lightx2v") {
            4
        } else {
            20
        },
        guidance: if distilled || model.contains("lightx2v") {
            1.0
        } else if ltx {
            3.0
        } else {
            3.5
        },
        shift: (!ltx).then_some(if model.contains("lightx2v") { 5.0 } else { 8.0 }),
        sampler: if ltx {
            if distilled {
                "euler_ancestral"
            } else {
                "euler"
            }
        } else {
            "euler"
        },
        scheduler: if model.starts_with("ltx25-") {
            "manual_sigmas"
        } else if ltx {
            "normal"
        } else {
            "simple"
        },
        min_frames: if ltx { 25 } else { 17 },
        max_frames: if ltx { 505 } else { 161 },
        dimension_step: if ltx { 64 } else { 16 },
    })
}

fn validate_ltx_fps(model: &str, fps: f64) -> Result<()> {
    if is_ltx_model(model) && (!fps.is_finite() || !(1.0..=60.0).contains(&fps)) {
        bail!("LTX FPS must be finite and between 1 and 60");
    }
    Ok(())
}

fn build(args: &Args) -> Result<ProjectRequest> {
    let config = spec(&args.model)?;
    let width = args.width.unwrap_or(config.width);
    let height = args.height.unwrap_or(config.height);
    if !width.is_multiple_of(config.dimension_step) || !height.is_multiple_of(config.dimension_step)
    {
        bail!(
            "dimensions must be divisible by {} for this model",
            config.dimension_step
        );
    }
    if args.duration <= 0.0 || args.batch == 0 || args.batch > 16 {
        bail!("duration must be positive and batch must be 1 through 16");
    }
    let fps = args.fps.unwrap_or(config.fps);
    validate_ltx_fps(&config.model, fps)?;
    if is_wan_model(&config.model) && ![16.0, 32.0].contains(&fps) {
        bail!("WAN 2.2 output FPS must be 16 or 32");
    }
    // WAN uses 16 generated fps even when output is interpolated to 32; LTX
    // generates at the requested FPS and follows its own frame grid.
    let frames = args.frames.unwrap_or(calculate_video_frames(
        &config.model,
        args.duration,
        fps,
        Some(config.min_frames),
        Some(config.max_frames),
    )?);
    if frames < config.min_frames || frames > config.max_frames {
        bail!("frame count is outside this model's supported range");
    }
    if args
        .audio_identity_strength
        .is_some_and(|strength| !(0.0..=1.0).contains(&strength))
    {
        bail!("audio identity strength must be 0 through 1");
    }
    if args.identity_audio.is_some() && !is_ltx_model(&config.model) {
        bail!("identity audio is supported only by LTX models");
    }
    let mut request = ProjectRequest::video(config.model, &args.prompt)
        .network(Network::Fast)
        .dimensions(width, height)
        .duration(args.duration)
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
        .param("billingMode", args.billing_mode.as_str());
    if let Some(value) = args.shift.or(config.shift) {
        request = request.param("shift", value);
    }
    if let Some(value) = &args.negative {
        request = request.param("negativePrompt", value.clone());
    }
    if let Some(value) = &args.style {
        request = request.param("stylePrompt", value.clone());
    }
    if let Some(value) = args.seed {
        request = request.param("seed", value);
    }
    if let Some(path) = &args.identity_audio {
        // Identity audio is a dedicated conditioning slot, not soundtrack input.
        require_file(path, "identity audio")?;
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

fn estimate(params: &Value) -> Value {
    json!({"tokenType": params["tokenType"], "model": params["modelId"], "width": params["width"],
        "height": params["height"], "frames": params["frames"], "fps": params["fps"],
        "steps": params["steps"], "numberOfMedia": params["numberOfMedia"]})
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let request = build(&args)?;
    if !execution_requested(args.execute, args.dry_run)? {
        print_request(&request)?;
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("sogni-rust-t2v"), Network::Fast).await?;
    let result = async {
        let model = request.params()["modelId"]
            .as_str()
            .expect("model")
            .to_owned();
        require_model(&client.projects, &model).await?;
        // Cost is quoted from the fully resolved request before any paid work.
        let quote = client
            .projects
            .estimate_video_cost(&estimate(&request.params()))
            .await?;
        confirm_estimate(&quote, args.yes)?;
        let project = client.projects.create(request).await?;
        println!("Project: {}", project.id());
        let urls = wait_with_progress(&project).await?;
        download_results(&urls, &args.output, "text-to-video", "mp4").await?;
        Result::<()>::Ok(())
    }
    .await;
    result.and(close(&client).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wan_32fps_keeps_internal_frame_count() {
        let args = Args::try_parse_from(["x", "--fps", "32"]).unwrap();
        assert_eq!(build(&args).unwrap().params()["frames"], 81);
    }
    #[test]
    fn ltx_uses_actual_fps_grid() {
        let args = Args::try_parse_from([
            "x",
            "--model",
            "ltx25-22b-int8_t2v_distilled",
            "--duration",
            "5",
            "--fps",
            "24",
        ])
        .unwrap();
        assert_eq!(build(&args).unwrap().params()["frames"], 121);
    }
    #[test]
    fn ltx_fps_boundaries_and_dry_run_are_validated() {
        let model = "ltx25-22b-int8_t2v_distilled";
        for fps in [1.0, 60.0] {
            assert!(validate_ltx_fps(model, fps).is_ok());
        }
        for fps in [0.0, 61.0, f64::NAN, f64::INFINITY] {
            assert!(validate_ltx_fps(model, fps).is_err());
        }
        assert!(validate_ltx_fps("seedance2", 100.0).is_ok());

        let args =
            Args::try_parse_from(["x", "--model", model, "--fps", "100", "--dry-run"]).unwrap();
        assert!(
            build(&args)
                .unwrap_err()
                .to_string()
                .contains("between 1 and 60")
        );
    }
}
