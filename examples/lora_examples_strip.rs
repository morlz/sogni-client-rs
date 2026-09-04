mod common;

use std::{collections::HashSet, env, fs, path::PathBuf, time::Instant};

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::json;
use sogni_client::{Network, ProjectRequest};

use common::{
    auth::{Credentials, close, connect_with_credentials, load_credentials, unique_app_id},
    cli::{execution_requested, explain_dry_run, prompt_text},
    files::{download, ensure_output_dir},
    progress::wait_with_progress,
    workflow::print_request,
};

#[derive(Debug, Parser)]
#[command(about = "Render a fixed-seed strength sweep for one Krea 2 LoRA")]
struct Args {
    #[arg(long, default_value = "krea2-amateur")]
    lora: String,
    #[arg(long, default_value = "-2,-1,off,1,2", allow_hyphen_values = true)]
    values: String,
    #[arg(long, default_value = "mark.and.worker")]
    worker: String,
    #[arg(long, default_value_t = 1_977_132_337)]
    seed: i64,
    #[arg(long, default_value = "krea2_turbo_fp8_scaled")]
    model: String,
    #[arg(long = "out", alias = "output")]
    output: Option<PathBuf>,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long)]
    prompt_file: Option<PathBuf>,
    #[arg(long)]
    negative_file: Option<PathBuf>,
    #[arg(long, default_value_t = 832)]
    width: u32,
    #[arg(long, default_value_t = 1216)]
    height: u32,
    #[arg(long)]
    disable_safe_content_filter: bool,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Clone, Debug)]
enum Strength {
    Off,
    Value(f64),
}

fn strengths(specification: &str) -> Result<Vec<Strength>> {
    let mut values = specification
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.eq_ignore_ascii_case("off") {
                Ok(Strength::Off)
            } else {
                let value = value.parse::<f64>()?;
                if !value.is_finite() {
                    bail!("LoRA strengths must be finite");
                }
                Ok(Strength::Value(value))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    values.sort_by(|left, right| numeric(left).total_cmp(&numeric(right)));
    if values.is_empty() {
        bail!("--values must contain at least one strength or off");
    }
    Ok(values)
}

fn numeric(value: &Strength) -> f64 {
    match value {
        Strength::Off => 0.0,
        Strength::Value(value) => *value,
    }
}

fn label(lora: &str, strength: &Strength) -> String {
    let suffix = match strength {
        Strength::Off => "off".into(),
        Strength::Value(value) if *value >= 0.0 => format!("pos{value}"),
        Strength::Value(value) => format!("neg{}", value.abs()),
    };
    format!("{lora}_{suffix}")
}

fn default_output() -> PathBuf {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads/krea2-lora-examples")
}

fn request(args: &Args, prompt: &str, negative: &str, strength: &Strength) -> ProjectRequest {
    let mut request =
        ProjectRequest::image(&args.model, format!("{prompt} --workers={}", args.worker))
            .number_of_media(1)
            .network(Network::Fast)
            .dimensions(args.width, args.height)
            .param("seed", args.seed)
            .param("tokenType", "spark")
            .param("sizePreset", "custom")
            .param("outputFormat", "png")
            .param("negativePrompt", negative)
            .param("disableNSFWFilter", args.disable_safe_content_filter);
    if let Strength::Value(value) = strength {
        request = request
            .param("loras", json!([&args.lora]))
            .param("loraStrengths", json!([value]));
    }
    request
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let prompt = prompt_text(args.prompt.clone(), args.prompt_file.as_deref(), "")?;
    let negative = match &args.negative_file {
        Some(path) => fs::read_to_string(path)?.trim().to_owned(),
        None => String::new(),
    };
    let strengths = strengths(&args.values)?;
    if args.width == 0 || args.height == 0 || args.width > 2560 || args.height > 2560 {
        bail!("--width and --height must be between 1 and 2560");
    }
    for strength in &strengths {
        println!("{}:", label(&args.lora, strength));
        print_request(&request(&args, &prompt, &negative, strength))?;
    }
    if !execution_requested(args.execute, args.dry_run)? {
        explain_dry_run();
        return Ok(());
    }
    let credentials = load_credentials()?;
    if !matches!(credentials, Credentials::ApiKey(_)) {
        bail!("this worker-pinning example requires SOGNI_API_KEY for a premium Spark account");
    }
    let client =
        connect_with_credentials(unique_app_id("lora-examples"), Network::Fast, credentials)
            .await?;
    let output = args.output.clone().unwrap_or_else(default_output);
    let result = async {
        ensure_output_dir(&output)?;
        let mut records = Vec::new();
        let mut workers = HashSet::new();
        for strength in &strengths {
            let label = label(&args.lora, strength);
            let started = Instant::now();
            let project = client.projects.create(request(&args, &prompt, &negative, strength)).await?;
            let urls = wait_with_progress(&project).await?;
            let url = urls.first().ok_or_else(|| anyhow::anyhow!("{label} returned no image"))?;
            let path = download(url, output.join(format!("{label}.png"))).await?;
            let worker = project.jobs().first().and_then(|job| job.snapshot().worker_name).unwrap_or_else(|| "unknown".into());
            workers.insert(worker.clone());
            println!("{label}: {:.1}s worker={worker} -> {}", started.elapsed().as_secs_f64(), path.display());
            let recorded_strength = match strength {
                Strength::Off => json!("off"),
                Strength::Value(value) => json!(value),
            };
            records.push(json!({"label": label, "strength": recorded_strength, "worker": worker, "file": path}));
        }
        let result_count = records.len();
        fs::write(output.join("strip.json"), serde_json::to_vec_pretty(&json!({"lora": args.lora, "seed": args.seed, "model": args.model, "results": records}))?)?;
        println!("{result_count} frames -> {}", output.display());
        if workers.len() == 1 {
            println!("All frames came from one worker: {}", workers.iter().next().unwrap());
        } else {
            eprintln!("Warning: frames came from mixed workers: {workers:?}");
        }
        Ok(())
    }.await;
    let close_result = close(&client).await;
    result.and(close_result)
}
