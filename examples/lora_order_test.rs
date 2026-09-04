//! Compare every ordering of a fixed Krea 2 LoRA stack at one seed.
//!
//! LoRA ids and strengths are positional arrays, so stack order is part of the
//! public request semantics and can change the result. This example generates
//! all permutations while holding prompt, model, seed, worker, and dimensions
//! constant, prints each request, and downloads one labeled PNG per ordering.
//!
//! The comparison requires an API key; reliable worker pinning also requires
//! Premium Spark eligibility, with placement remaining server-authoritative.
//! `--help` and dry-run permutation output need no credentials; paid rendering
//! requires `--execute` and an explicit render-count confirmation, or `--yes`.
//!
//! ```text
//! cargo run --example lora_order_test -- --help
//! cargo run --example lora_order_test -- --prompt-file prompt.txt --dry-run
//! cargo run --example lora_order_test -- --prompt-file prompt.txt --loras="krea2-detail-enhancer:3,krea2-amateur:-2" --execute
//! cargo run --example lora_order_test -- --prompt-file prompt.txt --out examples/output/order --execute --yes
//! ```

mod common;

use std::{env, path::PathBuf};

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::json;
use sogni_client::{Network, ProjectRequest};

use common::{
    auth::{Credentials, close, connect_with_credentials, load_credentials, unique_app_id},
    cli::{execution_requested, explain_dry_run, require_confirmation},
    files::{download, ensure_output_dir},
    lora::{LoraSetting, parse_stack, permutations},
    progress::wait_with_progress,
    workflow::print_request,
};

#[derive(Debug, Parser)]
#[command(about = "Compare every ordering of a fixed Krea 2 LoRA stack")]
struct Args {
    /// Text file containing the positive prompt.
    #[arg(long)]
    prompt_file: PathBuf,
    #[arg(long, default_value = "krea2-detail-enhancer:3,krea2-amateur:-2")]
    loras: String,
    #[arg(long, default_value_t = 1_977_132_337)]
    seed: i64,
    #[arg(long, default_value = "Allen,beeple,not.beeple")]
    worker: String,
    #[arg(long = "out")]
    output: Option<PathBuf>,
    #[arg(long, default_value = "krea2_turbo_fp8_scaled")]
    model: String,
    /// Submit the paid requests. Without this flag the example only prints them.
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    /// Skip the interactive render-count confirmation.
    #[arg(long)]
    yes: bool,
}

fn default_output() -> PathBuf {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads/lora-order-test")
}

fn request(args: &Args, prompt: &str, stack: &[LoraSetting]) -> ProjectRequest {
    // Build both arrays from the same ordered slice so ids cannot drift from strengths.
    ProjectRequest::image(&args.model, format!("{prompt} --workers={}", args.worker))
        .number_of_media(1)
        .network(Network::Fast)
        .dimensions(832, 1216)
        .param("seed", args.seed)
        .param("tokenType", "spark")
        .param("sizePreset", "custom")
        .param("outputFormat", "png")
        .param(
            "loras",
            json!(stack.iter().map(|item| &item.id).collect::<Vec<_>>()),
        )
        .param(
            "loraStrengths",
            json!(stack.iter().map(|item| item.strength).collect::<Vec<_>>()),
        )
}

fn label(stack: &[LoraSetting]) -> String {
    stack
        .iter()
        .map(|item| {
            format!(
                "{}@{}",
                item.id.strip_prefix("krea2-").unwrap_or(&item.id),
                item.strength
            )
        })
        .collect::<Vec<_>>()
        .join("__")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let prompt = std::fs::read_to_string(&args.prompt_file)?
        .trim()
        .to_owned();
    if prompt.is_empty() {
        bail!("--prompt-file must contain a non-empty prompt");
    }
    let stack = parse_stack(&args.loras, true)?;
    let permutations = permutations(&stack);
    println!(
        "{} permutation(s) of {} LoRAs, seed {}",
        permutations.len(),
        stack.len(),
        args.seed
    );
    for permutation in &permutations {
        println!("\n{}:", label(permutation));
        print_request(&request(&args, &prompt, permutation))?;
    }
    if !execution_requested(args.execute, args.dry_run)? {
        explain_dry_run();
        return Ok(());
    }
    require_confirmation(
        &format!("Submit {} paid render(s)?", permutations.len()),
        args.yes,
    )?;
    let credentials = load_credentials()?;
    if !matches!(credentials, Credentials::ApiKey(_)) {
        bail!("worker-pinned LoRA comparisons require SOGNI_API_KEY");
    }
    let client =
        connect_with_credentials(unique_app_id("lora-order"), Network::Fast, credentials).await?;
    let output = args.output.clone().unwrap_or_else(default_output);
    let result = async {
        ensure_output_dir(&output)?;
        for permutation in &permutations {
            let label = label(permutation);
            print!("{label:<46} ");
            let project = client
                .projects
                .create(request(&args, &prompt, permutation))
                .await?;
            let urls = wait_with_progress(&project).await?;
            let url = urls
                .first()
                .ok_or_else(|| anyhow::anyhow!("{label} returned no image"))?;
            let file = download(url, output.join(format!("{label}.png"))).await?;
            let worker = project
                .jobs()
                .first()
                .and_then(|job| job.snapshot().worker_name)
                .unwrap_or_else(|| "unknown".into());
            println!("worker={worker} -> {}", file.display());
        }
        println!("{} renders -> {}", permutations.len(), output.display());
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
    fn label_preserves_order_and_strength() {
        let stack = parse_stack("detail-enhancer:3,amateur:-2", true).unwrap();
        assert_eq!(label(&stack), "detail-enhancer@3__amateur@-2");
    }
}
