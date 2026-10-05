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
#[command(about = "Generate music with MiniMax Music 3 or ACE-Step 1.5")]
struct Args {
    #[arg(default_value = DEFAULT_PROMPT)]
    prompt: String,
    #[arg(long, help = "Canonical model ID; defaults to minimax_music3")]
    model: Option<String>,
    #[arg(long)]
    lyrics: Option<String>,
    #[arg(long, conflicts_with = "lyrics")]
    lyrics_file: Option<PathBuf>,
    #[arg(
        long,
        help = "Music 3: 10-300 seconds (default 60); ACE-Step: 10-600 (default 30)"
    )]
    duration: Option<f64>,
    #[arg(long, help = "ACE-Step tempo, or a Music 3 prompt direction")]
    bpm: Option<u16>,
    #[arg(long, help = "ACE-Step key, or a Music 3 prompt direction")]
    keyscale: Option<String>,
    #[arg(
        long,
        alias = "timesig",
        help = "ACE-Step meter, or a Music 3 prompt direction"
    )]
    timesignature: Option<String>,
    #[arg(long, help = "ACE-Step lyrics language, or a Music 3 prompt direction")]
    language: Option<String>,
    #[arg(long)]
    steps: Option<u32>,
    #[arg(long)]
    guidance: Option<f64>,
    #[arg(long, help = "ACE-Step only; selects ACE-Step if no model is named")]
    shift: Option<f64>,
    #[arg(long, help = "ACE-Step only; selects ACE-Step if no model is named")]
    no_composer_mode: bool,
    #[arg(
        long,
        help = "Prompt adherence 0-10; Music 3 defaults to 1.7, ACE-Step to 2"
    )]
    prompt_strength: Option<f64>,
    #[arg(long, help = "ACE-Step only; selects ACE-Step if no model is named")]
    creativity: Option<f64>,
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
    let model = resolved_model(args);
    let spec = model_spec(model)?;
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
    let duration = args.duration.unwrap_or(spec.duration_default);
    let mut prompt = args.prompt.clone();
    if !spec.ace_controls {
        let mut directions = Vec::new();
        if let Some(bpm) = args.bpm {
            directions.push(format!("Tempo: {bpm} BPM."));
        }
        if let Some(key) = &args.keyscale {
            directions.push(format!("Key: {key}."));
        }
        if let Some(signature) = &args.timesignature {
            directions.push(format!("Time signature: {signature}/4."));
        }
        if let Some(language) = &args.language {
            directions.push(format!("Lyrics language: {language}."));
        }
        if !directions.is_empty() {
            prompt = format!(
                "{}. {}",
                prompt.trim_end_matches([' ', '.']),
                directions.join(" ")
            );
        }
    }
    let lyrics = if !spec.ace_controls && lyrics.trim().is_empty() {
        "[Intro]\n[Verse]\n[Chorus]\n[Verse]\n[Chorus]\n[Bridge]\n[Outro]".to_owned()
    } else {
        lyrics
    };
    let mut request = ProjectRequest::audio(model, &prompt)
        .number_of_media(args.number)
        .steps(steps)
        .duration(duration)
        .param("lyrics", lyrics)
        .param(
            "promptStrength",
            args.prompt_strength.unwrap_or(spec.prompt_strength_default),
        )
        .param("sampler", sampler)
        .param("scheduler", scheduler)
        .param("outputFormat", args.format.clone())
        .param("tokenType", args.token_type.as_str())
        .param("billingMode", args.billing_mode.as_str());
    if spec.ace_controls {
        request = request
            .param("bpm", args.bpm.unwrap_or(120))
            .param("keyscale", args.keyscale.as_deref().unwrap_or("C major"))
            .param(
                "timesignature",
                args.timesignature.as_deref().unwrap_or("4"),
            )
            .param("language", args.language.as_deref().unwrap_or("en"))
            .param("shift", args.shift.unwrap_or(3.0))
            .param("composerMode", !args.no_composer_mode)
            .param("creativity", args.creativity.unwrap_or(0.85));
    }
    if let Some(guidance) = guidance {
        request = request.guidance(guidance);
    }
    if let Some(seed) = args.seed {
        request = request.param("seed", seed);
    }
    Ok(request)
}

fn resolved_model(args: &Args) -> &str {
    args.model.as_deref().unwrap_or_else(|| {
        if args.duration.is_some_and(|duration| duration > 300.0)
            || args.shift.is_some()
            || args.no_composer_mode
            || args.creativity.is_some()
        {
            "ace_step_1.5_xl_turbo"
        } else {
            "minimax_music3"
        }
    })
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
        let params = request.params();
        let model = params["modelId"].as_str().expect("music model was resolved");
        require_model(&client.projects, model).await?;
        // Estimate the resolved model duration, steps, and quantity before
        // creating a paid project.
        let estimate = client.projects.estimate_audio_cost(&json!({
            "tokenType": args.token_type.as_str(), "model": model,
            "duration": request.params()["duration"], "steps": request.params()["steps"], "numberOfMedia": args.number
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
        let turbo = Args::try_parse_from(["x", "--model", "ace_step_1.5_xl_turbo"]).unwrap();
        assert!(build(&turbo).unwrap().params().get("guidance").is_none());
        let turbo_with_guidance =
            Args::try_parse_from(["x", "--model", "ace_step_1.5_xl_turbo", "--guidance", "7"])
                .unwrap();
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

    #[test]
    fn default_music3_uses_its_own_defaults_and_omits_ace_fields() {
        let args = Args::try_parse_from(["x"]).unwrap();
        let request = build(&args).unwrap();
        let params = request.params();
        assert_eq!(params["modelId"], "minimax_music3");
        assert_eq!(params["duration"], 60.0);
        assert_eq!(params["steps"], 30);
        assert_eq!(params["guidance"], 1.7);
        assert_eq!(params["promptStrength"], 1.7);
        for key in [
            "bpm",
            "keyscale",
            "timesignature",
            "language",
            "shift",
            "composerMode",
            "creativity",
        ] {
            assert!(params.get(key).is_none(), "{key}");
        }
    }

    #[test]
    fn music3_directions_become_prompt_words_and_instrumental_lyrics_get_sections() {
        let args = Args::try_parse_from([
            "x",
            "lo-fi",
            "--bpm",
            "84",
            "--keyscale",
            "A minor",
            "--timesig",
            "4",
            "--language",
            "en",
            "--lyrics",
            "",
            "--seed",
            "0",
            "--prompt-strength",
            "0",
        ])
        .unwrap();
        let request = build(&args).unwrap();
        let params = request.params();
        assert_eq!(
            params["positivePrompt"],
            "lo-fi. Tempo: 84 BPM. Key: A minor. Time signature: 4/4. Lyrics language: en."
        );
        assert_eq!(
            params["lyrics"],
            "[Intro]\n[Verse]\n[Chorus]\n[Verse]\n[Chorus]\n[Bridge]\n[Outro]"
        );
        assert_eq!(params["seed"], 0);
        assert_eq!(params["promptStrength"], 0.0);
    }

    #[test]
    fn implicit_long_or_ace_only_settings_select_ace_and_explicit_music3_is_validated() {
        for flags in [
            vec!["x", "--duration", "420"],
            vec!["x", "--shift", "3"],
            vec!["x", "--no-composer-mode"],
            vec!["x", "--creativity", "0"],
        ] {
            let request = build(&Args::try_parse_from(flags).unwrap()).unwrap();
            assert_eq!(request.params()["modelId"], "ace_step_1.5_xl_turbo");
            assert!(request.params().get("guidance").is_none());
        }
        let explicit =
            Args::try_parse_from(["x", "--model", "minimax_music3", "--duration", "420"]).unwrap();
        assert_eq!(
            build(&explicit).unwrap_err().to_string(),
            "Duration must be between 10 and 300 seconds for MiniMax Music 3"
        );
        let explicit =
            Args::try_parse_from(["x", "--model", "minimax_music3", "--shift", "3"]).unwrap();
        assert_eq!(
            build(&explicit).unwrap_err().to_string(),
            "MiniMax Music 3 has no shift, composer-mode or creativity controls; use --model ace_step_1.5_xl_turbo"
        );
    }
}
