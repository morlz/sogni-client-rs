//! Render an ordered stack of up to eight bipolar Krea 2 LoRAs.
//!
//! Each `id:strength` entry becomes a pair of positional `loras` and
//! `loraStrengths` arrays. Positive and negative strengths can push an effect in
//! opposite directions, `0` is an explicit neutral value, and order remains
//! significant. `--reverse` renders the same prompt and seed with reversed stack
//! order for a controlled comparison. The example validates stack count, unique
//! ids, and finite strengths, estimates every paid render, and downloads PNGs.
//! Valid and recommended strength bands differ by LoRA; the service catalog and
//! server validation remain authoritative.
//!
//! `--help` and dry-run request inspection need no credentials. Live generation
//! requires credentials, `--execute`, and confirmation unless `--yes` is used.
//!
//! ```text
//! cargo run --example workflow_krea2_lora_stack -- --help
//! cargo run --example workflow_krea2_lora_stack -- "Editorial portrait at dusk" --dry-run
//! cargo run --example workflow_krea2_lora_stack -- "Editorial portrait" --loras="krea2-detail-enhancer:3,krea2-amateur:-2" --execute
//! cargo run --example workflow_krea2_lora_stack -- "Editorial portrait" --reverse --execute --yes
//! ```

mod common;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::json;
use sogni_client::{Network, ProjectRequest, ProjectsApi};

use common::{
    auth::{close, connect, unique_app_id},
    cli::{execution_requested, explain_dry_run},
    files::{download, ensure_output_dir, slug},
    lora::{LoraSetting, describe, parse_stack},
    progress::wait_with_progress,
    workflow::{estimate_and_confirm, print_request, require_model},
};

const DEFAULT_STACK: &str = "krea2-detail-enhancer:3,krea2-amateur:-2,krea2-warm-light:1.5";
const DEFAULT_PROMPT: &str = "A woman in her mid-20s with a short curly afro, wearing a bright yellow midi dress, crossing a city street at dusk, shot on 35mm film";

#[derive(Debug, Parser)]
#[command(
    about = "Stack up to eight bipolar Krea 2 LoRAs",
    long_about = "Render a Krea 2 LoRA stack in the supplied order. With --reverse, render the same stack and seed in reverse order for comparison."
)]
struct Args {
    /// Positive prompt; multiple words may be supplied without quoting.
    #[arg(value_name = "PROMPT", num_args = 0..)]
    prompt: Vec<String>,
    #[arg(long, default_value = DEFAULT_STACK, allow_hyphen_values = true)]
    loras: String,
    #[arg(long)]
    reverse: bool,
    #[arg(long, default_value = "krea2_turbo_fp8_scaled")]
    model: String,
    #[arg(long)]
    seed: Option<i64>,
    #[arg(long, default_value_t = 8)]
    steps: u32,
    #[arg(long, default_value_t = 832)]
    width: u32,
    #[arg(long, default_value_t = 1216)]
    height: u32,
    #[arg(long, default_value = "output")]
    output: PathBuf,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

fn request(args: &Args, prompt: &str, seed: i64, stack: &[LoraSetting]) -> ProjectRequest {
    // Derive paired positional arrays together; order is an intentional model input.
    ProjectRequest::image(&args.model, prompt)
        .number_of_media(1)
        .network(Network::Fast)
        .steps(args.steps)
        .dimensions(args.width, args.height)
        .param("seed", seed)
        .param("sizePreset", "custom")
        .param("outputFormat", "png")
        .param(
            "loras",
            json!(stack.iter().map(|entry| &entry.id).collect::<Vec<_>>()),
        )
        .param(
            "loraStrengths",
            json!(stack.iter().map(|entry| entry.strength).collect::<Vec<_>>()),
        )
}

async fn render(
    projects: &ProjectsApi,
    args: &Args,
    prompt: &str,
    seed: i64,
    stack: &[LoraSetting],
    label: &str,
) -> Result<PathBuf> {
    println!("\n{label}: {}", describe(stack));
    let request = request(args, prompt, seed, stack);
    estimate_and_confirm(projects, &request, args.yes).await?;
    let project = projects.create(request).await?;
    let urls = wait_with_progress(&project).await?;
    let url = urls
        .first()
        .ok_or_else(|| anyhow::anyhow!("{label} returned no image"))?;
    let filename = format!("krea2-stack-{}.png", slug(label, 48));
    let path = download(url, args.output.join(filename)).await?;
    println!("Saved {}", path.display());
    Ok(path)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let stack = parse_stack(&args.loras, false)?;
    if !(1..=u32::MAX).contains(&args.steps) {
        bail!("--steps must be positive");
    }
    if args.width == 0 || args.height == 0 {
        bail!("--width and --height must be positive");
    }
    let prompt = if args.prompt.is_empty() {
        DEFAULT_PROMPT.to_owned()
    } else {
        args.prompt.join(" ")
    };
    let seed = args.seed.unwrap_or_else(|| rand::random::<u32>() as i64);
    let mut variants = vec![("stack", stack.clone())];
    if args.reverse {
        // Keep the seed and strengths fixed so only stack order changes.
        variants.push(("reversed", stack.iter().cloned().rev().collect()));
    }
    for (label, stack) in &variants {
        println!("\n{label}: {}", describe(stack));
        print_request(&request(&args, &prompt, seed, stack))?;
    }
    if !execution_requested(args.execute, args.dry_run)? {
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("krea2-lora-stack"), Network::Fast).await?;
    let result = async {
        ensure_output_dir(&args.output)?;
        require_model(&client.projects, &args.model).await?;
        for (label, stack) in &variants {
            render(&client.projects, &args, &prompt, seed, stack, label).await?;
        }
        if args.reverse {
            println!("Both renders used the same seed and strengths; only LoRA order changed.");
        }
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
    fn request_keeps_lora_order() {
        let args = Args::parse_from(["test", "--loras", "first:1,second:-2"]);
        let stack = parse_stack(&args.loras, false).unwrap();
        let request = request(&args, "prompt", 7, &stack);
        assert_eq!(request.params()["loras"], json!(["first", "second"]));
        assert_eq!(request.params()["loraStrengths"], json!([1.0, -2.0]));
    }
}
