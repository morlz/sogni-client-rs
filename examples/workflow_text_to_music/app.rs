use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use serde_json::json;
use sogni_client::{Network, ProjectRequest};

mod validation;

use crate::common::{
    auth::{close, connect, unique_app_id},
    cli::{BillingMode, TokenType, confirm_estimate, execution_requested, explain_dry_run},
    files::download_results,
    progress::wait_with_progress,
    workflow::{print_request, require_model},
};
use validation::{model_spec, validate};

const DEFAULT_PROMPT: &str = "Robotic vocoder electro-anthem, French house and hip-hop energy, talkbox lead vocal, crunchy synth stabs, four-on-the-floor disco beat.";
const DEFAULT_LYRICS: &str =
    "[Intro]\nSYSTEM ONLINE.\n\n[Chorus]\nRender faster.\nDreams louder.\nSogni stronger.";

#[derive(Debug, Parser)]
#[command(about = "Generate music with ACE-Step 1.5")]
struct Args {
    #[arg(default_value = DEFAULT_PROMPT)]
    prompt: String,
    #[arg(long, default_value = "ace_step_1.5_xl_turbo")]
    model: String,
    #[arg(long)]
    lyrics: Option<String>,
    #[arg(long, conflicts_with = "lyrics")]
    lyrics_file: Option<PathBuf>,
    #[arg(long, default_value_t = 30.0)]
    duration: f64,
    #[arg(long, default_value_t = 120)]
    bpm: u16,
    #[arg(long, default_value = "C major")]
    keyscale: String,
    #[arg(long, default_value = "4")]
    timesignature: String,
    #[arg(long, default_value = "en")]
    language: String,
    #[arg(long)]
    steps: Option<u32>,
    #[arg(long)]
    guidance: Option<f64>,
    #[arg(long, default_value_t = 3.0)]
    shift: f64,
    #[arg(long = "no-composer-mode", action = clap::ArgAction::SetFalse, default_value_t = true)]
    composer_mode: bool,
    #[arg(long, default_value_t = 2.0)]
    prompt_strength: f64,
    #[arg(long, default_value_t = 0.85)]
    creativity: f64,
    #[arg(long)]
    sampler: Option<String>,
    #[arg(long)]
    scheduler: Option<String>,
    #[arg(long)]
    seed: Option<u32>,
    #[arg(long, default_value = "mp3")]
    format: String,
    #[arg(long = "batch", alias = "number", default_value_t = 1)]
    number: u32,
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

fn build(args: &Args) -> Result<ProjectRequest> {
    let spec = model_spec(&args.model)?;
    let steps = args.steps.unwrap_or(spec.steps);
    let lyrics = match (&args.lyrics, &args.lyrics_file) {
        (Some(value), _) => value.clone(),
        (_, Some(path)) => {
            fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
        }
        _ => DEFAULT_LYRICS.to_owned(),
    };
    // `None` is meaningful for Turbo: never serialize CFG just because the
    // caller supplied a value that this model cannot consume.
    let guidance = spec
        .guidance
        .map(|default| args.guidance.unwrap_or(default));
    let sampler = args.sampler.as_deref().unwrap_or(spec.sampler);
    let scheduler = args.scheduler.as_deref().unwrap_or(spec.scheduler);
    validate(args, &spec, &lyrics, steps, guidance, sampler, scheduler)?;
    if spec.guidance.is_none() && args.guidance.is_some() {
        eprintln!(
            "Warning: {} does not use CFG guidance, ignoring --guidance",
            spec.name
        );
    }
    let mut request = ProjectRequest::audio(&args.model, &args.prompt)
        .number_of_media(args.number)
        .steps(steps)
        .duration(args.duration)
        .param("bpm", args.bpm)
        .param("keyscale", args.keyscale.clone())
        .param("timesignature", args.timesignature.clone())
        .param("language", args.language.clone())
        .param("lyrics", lyrics)
        .param("shift", args.shift)
        .param("composerMode", args.composer_mode)
        .param("promptStrength", args.prompt_strength)
        .param("creativity", args.creativity)
        .param("sampler", sampler)
        .param("scheduler", scheduler)
        .param("outputFormat", args.format.clone())
        .param("tokenType", args.token_type.as_str())
        .param("billingMode", args.billing_mode.as_str());
    if let Some(guidance) = guidance {
        request = request.guidance(guidance);
    }
    if let Some(seed) = args.seed {
        request = request.param("seed", seed);
    }
    Ok(request)
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let request = build(&args)?;
    if !execution_requested(args.execute, args.dry_run)? {
        print_request(&request)?;
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("sogni-rust-text-music"), Network::Fast).await?;
    let result = async {
        require_model(&client.projects, &args.model).await?;
        // Estimate the resolved model duration, steps, and quantity before
        // creating a paid project.
        let estimate = client.projects.estimate_audio_cost(&json!({
            "tokenType": args.token_type.as_str(), "model": args.model,
            "duration": args.duration, "steps": request.params()["steps"], "numberOfMedia": args.number
        })).await?;
        confirm_estimate(&estimate, args.yes)?;
        let project = client.projects.create(request).await?;
        println!("Project: {}", project.id());
        let urls = wait_with_progress(&project).await?;
        download_results(&urls, &args.output, "music", &args.format).await?;
        Result::<()>::Ok(())
    }.await;
    result.and(close(&client).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn turbo_omits_cfg_and_sft_defaults_it() {
        let turbo = Args::try_parse_from(["x"]).unwrap();
        assert!(build(&turbo).unwrap().params().get("guidance").is_none());
        let turbo_with_guidance = Args::try_parse_from(["x", "--guidance", "7"]).unwrap();
        assert!(
            build(&turbo_with_guidance)
                .unwrap()
                .params()
                .get("guidance")
                .is_none()
        );
        let sft = Args::try_parse_from(["x", "--model", "ace_step_1.5_xl_sft"]).unwrap();
        assert_eq!(build(&sft).unwrap().params()["guidance"], 7.0);
    }
}
