//! Generate a controlled camera view with the Multiple Angles LoRA.
//!
//! Eight azimuths, four elevations, and three distances form 96 camera-pose
//! combinations. The required `<sks>` activation token is placed first, followed
//! by the selected camera terms and an optional subject anchor. One context image
//! occupies slot 1; the LoRA id and strength remain a positional pair. Both the
//! fast and quality Qwen edit profiles retain their own steps/guidance defaults.
//!
//! `--help` is credential-free. Dry runs still validate/read the reference image
//! but do not connect. A paid render requires credentials, `--execute`, and cost
//! confirmation unless `--yes` is passed; the generated prompt is printed and
//! JPEG results are downloaded to `examples/output` by default.
//!
//! ```text
//! cargo run --example workflow_multiple_angles -- --help
//! cargo run --example workflow_multiple_angles -- --context subject.jpg --azimuth back --elevation eye-level --distance medium --dry-run
//! cargo run --example workflow_multiple_angles -- "woman in a red coat" --context subject.jpg --model qwen-lightning --execute
//! cargo run --example workflow_multiple_angles -- --context subject.jpg --azimuth front-right --elevation low-angle --distance close-up --execute --yes
//! ```

mod common;
#[path = "workflow_multiple_angles/config.rs"]
mod config;

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
use config::{Args, Azimuth, Distance, Elevation, camera_prompt};

const LORA_ID: &str = "multiple_angles";

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
        // LoRA ids and strengths form one positional pair; reordering either changes meaning.
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
