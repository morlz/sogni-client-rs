mod common;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use serde_json::json;
use sogni_client::{AssetRole, MediaSource, Network, ProjectRequest};

use common::{
    auth::{close, connect, unique_app_id},
    cli::{
        BillingMode, TokenType, execution_requested, explain_dry_run, resolve_billing_mode,
        resolve_token_type, select,
    },
    files::{choose_file, slug},
    models::{edit_model, validate_image_options},
    workflow::{estimate_and_confirm, print_request, require_model, run_image_project},
};

const LORA_ID: &str = "multiple_angles";

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Azimuth {
    #[default]
    Front,
    FrontRight,
    Right,
    BackRight,
    Back,
    BackLeft,
    Left,
    FrontLeft,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Elevation {
    LowAngle,
    #[default]
    EyeLevel,
    Elevated,
    HighAngle,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Distance {
    CloseUp,
    #[default]
    Medium,
    Wide,
}

impl Azimuth {
    const fn prompt(self) -> &'static str {
        match self {
            Self::Front => "front view",
            Self::FrontRight => "front-right quarter view",
            Self::Right => "right side view",
            Self::BackRight => "back-right quarter view",
            Self::Back => "back view",
            Self::BackLeft => "back-left quarter view",
            Self::Left => "left side view",
            Self::FrontLeft => "front-left quarter view",
        }
    }
}

impl Elevation {
    const fn prompt(self) -> &'static str {
        match self {
            Self::LowAngle => "low-angle shot",
            Self::EyeLevel => "eye-level shot",
            Self::Elevated => "elevated shot",
            Self::HighAngle => "high-angle shot",
        }
    }
}

impl Distance {
    const fn prompt(self) -> &'static str {
        match self {
            Self::CloseUp => "close-up",
            Self::Medium => "medium shot",
            Self::Wide => "wide shot",
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Generate one of 96 camera poses with the Multiple Angles LoRA")]
struct Args {
    #[arg(value_name = "DESCRIPTION")]
    description: Option<String>,
    #[arg(long, alias = "image")]
    context: Option<PathBuf>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, value_enum)]
    azimuth: Option<Azimuth>,
    #[arg(long, value_enum)]
    elevation: Option<Elevation>,
    #[arg(long, value_enum)]
    distance: Option<Distance>,
    #[arg(long, default_value_t = 0.9)]
    strength: f64,
    #[arg(long, alias = "anchor")]
    anchor: Option<String>,
    #[arg(long)]
    guidance: Option<f64>,
    #[arg(long, default_value_t = 1024)]
    width: u32,
    #[arg(long, default_value_t = 1024)]
    height: u32,
    #[arg(long, default_value_t = 1)]
    batch: u32,
    #[arg(long)]
    seed: Option<i64>,
    #[arg(long)]
    steps: Option<u32>,
    #[arg(long, default_value = "examples/output")]
    output: PathBuf,
    #[arg(long)]
    disable_safe_content_filter: bool,
    #[arg(long)]
    no_interactive: bool,
    #[arg(long, alias = "billing", value_enum)]
    billing_mode: Option<BillingMode>,
    #[arg(long, value_enum)]
    token_type: Option<TokenType>,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

fn camera_prompt(
    azimuth: Azimuth,
    elevation: Elevation,
    distance: Distance,
    anchor: Option<&str>,
) -> String {
    let mut prompt = format!(
        "<sks> {} {} {}",
        azimuth.prompt(),
        elevation.prompt(),
        distance.prompt()
    );
    if let Some(anchor) = anchor.filter(|value| !value.trim().is_empty()) {
        prompt.push(' ');
        prompt.push_str(anchor.trim());
    }
    prompt
}

struct Render<'a> {
    model_id: &'a str,
    image: &'a std::path::Path,
    prompt: &'a str,
    steps: u32,
    guidance: f64,
    seed: i64,
    token: TokenType,
    billing: BillingMode,
}

fn request(args: &Args, render: &Render<'_>) -> ProjectRequest {
    ProjectRequest::image(render.model_id, render.prompt)
        .network(Network::Fast)
        .number_of_media(args.batch)
        .dimensions(args.width, args.height)
        .steps(render.steps)
        .guidance(render.guidance)
        .asset(
            AssetRole::ContextImage(1),
            MediaSource::Path(render.image.to_owned()),
        )
        .param("seed", render.seed)
        .param("tokenType", render.token.as_str())
        .param("billingMode", render.billing.as_str())
        .param("sizePreset", "custom")
        .param("outputFormat", "jpg")
        .param("sampler", "euler")
        .param("scheduler", "simple")
        .param("loras", json!([LORA_ID]))
        .param("loraStrengths", json!([args.strength]))
        .param("disableNSFWFilter", args.disable_safe_content_filter)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let interactive = !args.no_interactive;
    let model_key = match &args.model {
        Some(key) => key.clone(),
        None if interactive => select(
            "Select a Multiple Angles model",
            &[
                ("qwen-lightning", "fast 4-step generation"),
                ("qwen", "higher-quality 20-step generation"),
            ],
            "qwen-lightning",
        )?,
        None => "qwen-lightning".into(),
    };
    if !matches!(model_key.as_str(), "qwen-lightning" | "qwen") {
        bail!("--model must be qwen-lightning or qwen");
    }
    let model = edit_model(&model_key)?;
    let image = choose_file(args.context.clone(), "reference image", interactive)?;
    if !(0.0..=1.5).contains(&args.strength) {
        bail!("--strength must be between 0 and 1.5");
    }
    if !(1..=512).contains(&args.batch) {
        bail!("--batch must be between 1 and 512");
    }
    let steps = args.steps.unwrap_or(model.default_steps);
    let guidance = args.guidance.unwrap_or(model.default_guidance);
    validate_image_options(model, args.width, args.height, steps, guidance)?;
    let azimuth = match args.azimuth {
        Some(value) => value,
        None if interactive => Azimuth::from_str(
            &select(
                "Azimuth",
                &[
                    ("front", "front view"),
                    ("front-right", "front-right quarter"),
                    ("right", "right side"),
                    ("back-right", "back-right quarter"),
                    ("back", "back view"),
                    ("back-left", "back-left quarter"),
                    ("left", "left side"),
                    ("front-left", "front-left quarter"),
                ],
                "front",
            )?,
            true,
        )
        .map_err(|_| anyhow::anyhow!("invalid azimuth"))?,
        None => Azimuth::default(),
    };
    let elevation = match args.elevation {
        Some(value) => value,
        None if interactive => Elevation::from_str(
            &select(
                "Elevation",
                &[
                    ("low-angle", "low angle (-30 degrees)"),
                    ("eye-level", "eye level"),
                    ("elevated", "elevated (30 degrees)"),
                    ("high-angle", "high angle (60 degrees)"),
                ],
                "eye-level",
            )?,
            true,
        )
        .map_err(|_| anyhow::anyhow!("invalid elevation"))?,
        None => Elevation::default(),
    };
    let distance = match args.distance {
        Some(value) => value,
        None if interactive => Distance::from_str(
            &select(
                "Distance",
                &[
                    ("close-up", "close-up (0.6x)"),
                    ("medium", "medium shot"),
                    ("wide", "wide shot (1.8x)"),
                ],
                "medium",
            )?,
            true,
        )
        .map_err(|_| anyhow::anyhow!("invalid distance"))?,
        None => Distance::default(),
    };
    let anchor = match args.anchor.clone().or_else(|| args.description.clone()) {
        Some(value) => value,
        None if interactive => common::cli::prompt(
            "Optional anchor description (leave blank to skip)",
            Some(""),
        )?,
        None => String::new(),
    };
    let prompt = camera_prompt(azimuth, elevation, distance, Some(&anchor));
    let seed = args
        .seed
        .filter(|seed| *seed != -1)
        .unwrap_or_else(|| i64::from(rand::random::<u32>() & i32::MAX as u32));
    let token = resolve_token_type(args.token_type, interactive && args.execute)?;
    let billing = resolve_billing_mode(args.billing_mode)?;
    let render = Render {
        model_id: model.id,
        image: &image,
        prompt: &prompt,
        steps,
        guidance,
        seed,
        token,
        billing,
    };
    let request = request(&args, &render);
    println!("Generated prompt: {prompt}");
    print_request(&request)?;
    if !execution_requested(args.execute, args.dry_run)? {
        explain_dry_run();
        return Ok(());
    }
    let client = connect(
        unique_app_id("sogni-workflow-multiple-angles"),
        Network::Fast,
    )
    .await?;
    let result = async {
        require_model(&client.projects, model.id).await?;
        estimate_and_confirm(&client.projects, &request, args.yes).await?;
        let prefix = format!("multiple-angles-{seed}-{}", slug(&prompt, 48));
        run_image_project(&client.projects, request, &args.output, &prefix, "jpg").await?;
        Ok(())
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_places_activation_keyword_first() {
        assert_eq!(
            camera_prompt(
                Azimuth::Back,
                Elevation::HighAngle,
                Distance::CloseUp,
                Some("red coat")
            ),
            "<sks> back view high-angle shot close-up red coat"
        );
    }
}
