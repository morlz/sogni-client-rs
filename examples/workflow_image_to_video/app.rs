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

const DEFAULT_PROMPT: &str = "A retro diner waitress smiles warmly as the camera slowly pushes toward her face; ambient dishes, conversation, and a jukebox accompany her greeting.";

#[derive(Debug, Parser)]
#[command(about = "Animate a first frame, optionally ending on a second frame")]
struct Args {
    #[arg(default_value = DEFAULT_PROMPT)]
    prompt: String,
    #[arg(long, default_value = "test-assets/placeholder6.jpg")]
    image: PathBuf,
    #[arg(long)]
    end_image: Option<PathBuf>,
    #[arg(long, default_value = "wan_v2.2-14b-fp8_i2v_lightx2v")]
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
    #[arg(long = "first-frame-strength", default_value_t = 0.7)]
    first_strength: f64,
    #[arg(long = "last-frame-strength", default_value_t = 0.7)]
    last_strength: f64,
    #[arg(long)]
    transition: bool,
    #[arg(long, requires = "transition", default_value_t = 1.0)]
    transition_strength: f64,
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

struct Spec {
    id: String,
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
    grid: u32,
}

fn spec(value: &str) -> Result<Spec> {
    let id = match value {
        "lightx2v" => "wan_v2.2-14b-fp8_i2v_lightx2v",
        "quality" => "wan_v2.2-14b-fp8_i2v",
        other => other,
    };
    if get_video_workflow_type(id) != Some("i2v") {
        bail!("model {id} is not image-to-video");
    }
    let ltx = is_ltx_model(id);
    let distilled = id.contains("distilled");
    Ok(Spec {
        id: id.into(),
        width: if ltx { 1920 } else { 640 },
        height: if ltx { 1088 } else { 640 },
        fps: if ltx { 24.0 } else { 16.0 },
        steps: if id.contains("10eros") {
            9
        } else if ltx {
            if distilled { 8 } else { 30 }
        } else if id.contains("lightx2v") {
            4
        } else {
            20
        },
        guidance: if distilled || id.contains("10eros") || id.contains("lightx2v") {
            1.0
        } else if ltx {
            3.0
        } else {
            4.0
        },
        shift: (!ltx).then_some(if id.contains("lightx2v") { 5.0 } else { 8.0 }),
        sampler: if ltx { "euler_ancestral" } else { "euler" },
        scheduler: if id.starts_with("ltx25-") || id.contains("10eros") {
            "manual_sigmas"
        } else if ltx {
            "normal"
        } else {
            "simple"
        },
        min_frames: if ltx { 25 } else { 17 },
        max_frames: if ltx { 505 } else { 161 },
        grid: if ltx { 64 } else { 16 },
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
    let ltx = is_ltx_model(&config.id);
    if config.id.contains("10eros") && !args.disable_safe_content_filter {
        bail!("the 10Eros model requires --disable-safe-content-filter");
    }
    if args.transition && (!ltx || args.end_image.is_none()) {
        bail!("--transition requires an LTX I2V model and --end-image");
    }
    if !(0.0..=2.0).contains(&args.transition_strength) {
        bail!("transition strength must be 0 through 2");
    }
    for (value, label) in [(args.first_strength, "first"), (args.last_strength, "last")] {
        if !(0.0..=1.0).contains(&value) {
            bail!("{label}-frame strength must be 0 through 1");
        }
    }
    let width = args.width.unwrap_or(config.width);
    let height = args.height.unwrap_or(config.height);
    if width % config.grid != 0 || height % config.grid != 0 {
        bail!("dimensions must be divisible by {}", config.grid);
    }
    let fps = args.fps.unwrap_or(config.fps);
    validate_ltx_fps(&config.id, fps)?;
    if is_wan_model(&config.id) && ![16.0, 32.0].contains(&fps) {
        bail!("WAN FPS must be 16 or 32");
    }
    let frames = args.frames.unwrap_or(calculate_video_frames(
        &config.id,
        args.duration,
        fps,
        Some(config.min_frames),
        Some(config.max_frames),
    )?);
    let prompt = if args.transition && !args.prompt.to_ascii_lowercase().contains("zhuanchang") {
        format!("{} zhuanchang", args.prompt)
    } else {
        args.prompt.clone()
    };
    let mut request = ProjectRequest::video(config.id, prompt)
        .network(Network::Fast)
        .dimensions(width, height)
        .duration(args.duration)
        .fps(fps)
        .number_of_media(args.batch)
        .steps(args.steps.unwrap_or(config.steps))
        .guidance(args.guidance.unwrap_or(config.guidance))
        .param("frames", frames)
        .param("firstFrameStrength", args.first_strength)
        .param("lastFrameStrength", args.last_strength)
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
        .asset(
            AssetRole::ReferenceImage,
            MediaSource::Path(args.image.clone()),
        );
    if let Some(value) = args.shift.or(config.shift) {
        request = request.param("shift", value);
    }
    if let Some(value) = args.seed {
        request = request.param("seed", value);
    }
    if let Some(value) = &args.negative {
        request = request.param("negativePrompt", value.clone());
    }
    if let Some(value) = &args.style {
        request = request.param("stylePrompt", value.clone());
    }
    if let Some(path) = &args.end_image {
        request = request.asset(
            AssetRole::ReferenceImageEnd,
            MediaSource::Path(path.clone()),
        );
    }
    if args.transition {
        request = request
            .param("loras", json!(["transition"]))
            .param("loraStrengths", json!([args.transition_strength]));
    }
    if let Some(path) = &args.identity_audio {
        if !ltx {
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

fn require_inputs(args: &Args) -> Result<()> {
    require_file(&args.image, "starting image")?;
    if let Some(path) = &args.end_image {
        require_file(path, "ending image")?;
    }
    if let Some(path) = &args.identity_audio {
        require_file(path, "identity audio")?;
    }
    Ok(())
}

fn estimate(params: &Value) -> Value {
    json!({"tokenType":params["tokenType"],"model":params["modelId"],"width":params["width"],"height":params["height"],"frames":params["frames"],"fps":params["fps"],"steps":params["steps"],"numberOfMedia":params["numberOfMedia"]})
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let request = build(&args)?;
    if !execution_requested(args.execute, args.dry_run)? {
        print_request(&request)?;
        explain_dry_run();
        return Ok(());
    }
    require_inputs(&args)?;
    let client = connect(unique_app_id("sogni-rust-i2v"), Network::Fast).await?;
    let result = async {
        let id = request.params()["modelId"]
            .as_str()
            .expect("model")
            .to_owned();
        require_model(&client.projects, &id).await?;
        let quote = client
            .projects
            .estimate_video_cost(&estimate(&request.params()))
            .await?;
        confirm_estimate(&quote, args.yes)?;
        let project = client.projects.create(request).await?;
        println!("Project: {}", project.id());
        let urls = wait_with_progress(&project).await?;
        download_results(&urls, &args.output, "image-to-video", "mp4").await?;
        Result::<()>::Ok(())
    }
    .await;
    result.and(close(&client).await)
}

#[cfg(test)]
mod tests;
