use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use sogni_client::{
    AssetRole, MediaSource, Network, ProjectRequest, calculate_video_frames,
    get_video_workflow_type, is_ltx_model, is_wan_model,
};

use crate::common::{
    auth::{close, connect, unique_app_id},
    cli::{BillingMode, TokenType, execution_requested, explain_dry_run, require_confirmation},
    files::{download, ensure_output_dir, image_dimensions},
    progress::wait_with_progress,
    workflow::{print_request, require_model},
};

const DEFAULT_PROMPT: &str =
    "A cinematic camera movement that brings the image to life with smooth, natural motion";

#[derive(Debug, Parser)]
#[command(about = "Animate every image in a folder with one I2V configuration")]
struct Args {
    #[arg(default_value = DEFAULT_PROMPT)]
    prompt: String,
    #[arg(long, default_value = "toprocess")]
    folder: PathBuf,
    #[arg(long, default_value = "lightx2v")]
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
    #[arg(long, default_value_t = true, action = clap::ArgAction::SetTrue)]
    skip_existing: bool,
    #[arg(long = "no-skip-existing")]
    no_skip_existing: bool,
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
    fps: f64,
    steps: u32,
    guidance: f64,
    shift: Option<f64>,
    sampler: &'static str,
    scheduler: &'static str,
    grid: u32,
    min: i64,
    max: i64,
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
    let fast = id.contains("distilled") || id.contains("lightx2v");
    Ok(Spec {
        id: id.into(),
        fps: if ltx { 24.0 } else { 16.0 },
        steps: if ltx {
            if fast { 8 } else { 30 }
        } else if fast {
            4
        } else {
            20
        },
        guidance: if fast {
            1.0
        } else if ltx {
            3.0
        } else {
            4.0
        },
        shift: (!ltx).then_some(if fast { 5.0 } else { 8.0 }),
        sampler: if ltx { "euler_ancestral" } else { "euler" },
        scheduler: if id.starts_with("ltx25-") {
            "manual_sigmas"
        } else if ltx {
            "normal"
        } else {
            "simple"
        },
        grid: if ltx { 64 } else { 16 },
        min: if ltx { 25 } else { 17 },
        max: if ltx { 505 } else { 161 },
    })
}

fn validate_ltx_fps(model: &str, fps: f64) -> Result<()> {
    if is_ltx_model(model) && (!fps.is_finite() || !(1.0..=60.0).contains(&fps)) {
        bail!("LTX FPS must be finite and between 1 and 60");
    }
    Ok(())
}

fn images(folder: &Path) -> Result<Vec<PathBuf>> {
    if !folder.is_dir() {
        bail!("input folder does not exist: {}", folder.display());
    }
    let mut files = fs::read_dir(folder)
        .with_context(|| format!("read {}", folder.display()))?
        .filter_map(|entry| entry.ok().map(|v| v.path()))
        .filter(|path| {
            path.extension().and_then(|v| v.to_str()).is_some_and(|v| {
                matches!(
                    v.to_ascii_lowercase().as_str(),
                    "jpg" | "jpeg" | "png" | "webp"
                )
            })
        })
        .collect::<Vec<_>>();
    files.sort();
    if files.is_empty() {
        bail!("no JPEG, PNG, or WebP images found in {}", folder.display());
    }
    Ok(files)
}

fn rounded_dimension(value: u32, grid: u32, min: u32, max: u32) -> u32 {
    ((value.clamp(min, max) + grid / 2) / grid * grid).clamp(min, max)
}
fn build(args: &Args, image: &Path, inferred: Option<(u32, u32)>) -> Result<ProjectRequest> {
    let config = spec(&args.model)?;
    let source = inferred.unwrap_or((640, 640));
    let min = if is_ltx_model(&config.id) { 640 } else { 480 };
    let max = if is_ltx_model(&config.id) { 3840 } else { 1536 };
    let width = args
        .width
        .unwrap_or_else(|| rounded_dimension(source.0, config.grid, min, max));
    let height = args
        .height
        .unwrap_or_else(|| rounded_dimension(source.1, config.grid, min, max));
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
        Some(config.min),
        Some(config.max),
    )?);
    let mut request = ProjectRequest::video(config.id, &args.prompt)
        .network(Network::Fast)
        .dimensions(width, height)
        .duration(args.duration)
        .fps(fps)
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
        .asset(
            AssetRole::ReferenceImage,
            MediaSource::Path(image.to_owned()),
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
    Ok(request)
}
fn estimate(params: &Value) -> Value {
    json!({"tokenType":params["tokenType"],"model":params["modelId"],"width":params["width"],"height":params["height"],"frames":params["frames"],"fps":params["fps"],"steps":params["steps"],"numberOfMedia":1})
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let execute = execution_requested(args.execute, args.dry_run)?;
    let inputs = if !execute && !args.folder.is_dir() {
        vec![args.folder.join("example.png")]
    } else {
        images(&args.folder)?
    };
    let inferred = inputs
        .first()
        .filter(|p| p.is_file())
        .and_then(|p| image_dimensions(p).ok());
    let sample = build(&args, &inputs[0], inferred)?;
    if !execute {
        println!("Batch contains {} input(s). Sample request:", inputs.len());
        print_request(&sample)?;
        explain_dry_run();
        return Ok(());
    }
    ensure_output_dir(&args.output)?;
    let client = connect(unique_app_id("sogni-rust-batch-i2v"), Network::Fast).await?;
    let result = async {
        let model = sample.params()["modelId"]
            .as_str()
            .expect("model")
            .to_owned();
        require_model(&client.projects, &model).await?;
        let quote = client
            .projects
            .estimate_video_cost(&estimate(&sample.params()))
            .await?;
        println!(
            "Per-video estimate: {} Spark; {} video(s) selected.",
            quote.spark,
            inputs.len()
        );
        require_confirmation("Submit the paid batch?", args.yes)?;
        for (index, input) in inputs.iter().enumerate() {
            let stem = input
                .file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("video");
            let destination = args.output.join(format!("{stem}.mp4"));
            if args.skip_existing && !args.no_skip_existing && destination.exists() {
                println!("Skipping existing {}", destination.display());
                continue;
            }
            let request = build(&args, input, image_dimensions(input).ok())?;
            let project = client.projects.create(request).await?;
            println!(
                "[{}/{}] Project {} for {}",
                index + 1,
                inputs.len(),
                project.id(),
                input.display()
            );
            let urls = wait_with_progress(&project).await?;
            let url = urls
                .first()
                .ok_or_else(|| anyhow::anyhow!("project completed without a result"))?;
            let saved = download(url, destination).await?;
            println!("Saved {}", saved.display());
        }
        Result::<()>::Ok(())
    }
    .await;
    result.and(close(&client).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_keep_wan_frame_contract() {
        let args = Args::try_parse_from(["x", "--fps", "32"]).unwrap();
        let p = build(&args, Path::new("x.png"), Some((640, 640)))
            .unwrap()
            .params();
        assert_eq!(p["modelId"], "wan_v2.2-14b-fp8_i2v_lightx2v");
        assert_eq!(p["frames"], 81);
    }
    #[test]
    fn ltx_fps_boundaries_and_dry_run_are_validated() {
        let model = "ltx25-22b-int8_i2v_distilled";
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
            build(&args, Path::new("x.png"), Some((640, 640)))
                .unwrap_err()
                .to_string()
                .contains("between 1 and 60")
        );
    }
}
