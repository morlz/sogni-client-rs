//! Generate an image while observing Sogni's event-driven project lifecycle.
//!
//! This is the Rust counterpart of the SDK event example: it discovers a live
//! image model, estimates the request, then reports API-wide and project-local
//! job, progress, completion, and failure events. The project completion result
//! remains authoritative; event streams provide incremental observability.
//!
//! `--help` and dry-run request rendering are credential-free. A paid run needs
//! Sogni credentials plus `--execute`, and asks for confirmation after the cost
//! estimate unless `--yes` is present. The example prints result URLs rather
//! than downloading them.
//!
//! ```text
//! cargo run --example event_driven -- --help
//! cargo run --example event_driven -- --prompt "A fox in a glass forest" --dry-run
//! cargo run --example event_driven -- --prompt "A fox in a glass forest" --execute
//! cargo run --example event_driven -- --execute --yes
//! ```

mod common;

use std::time::Duration;

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::Value;
use sogni_client::{Network, ProjectRequest};

use common::{
    auth::{close, connect, unique_app_id},
    cli::{confirm_estimate, execution_requested, explain_dry_run},
    progress::{spawn_api_reporter, spawn_project_reporter},
    workflow::{estimate_image, print_request},
};

#[derive(Debug, Parser)]
#[command(about = "Track image generation through project and job event streams")]
struct Args {
    #[arg(long, default_value = "A cat wearing a hat")]
    prompt: String,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

fn request(model_id: &str, prompt: &str) -> ProjectRequest {
    ProjectRequest::image(model_id, prompt)
        .steps(20)
        .guidance(7.5)
        .number_of_media(2)
        .network(Network::Fast)
        .param("negativePrompt", "malformation, bad anatomy, bad hands, missing fingers, cropped, low quality, bad quality, jpeg artifacts, watermark")
        .param("stylePrompt", "anime")
        .param("numberOfPreviews", 2)
        .param("outputFormat", "png")
        .param("tokenType", "spark")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if !execution_requested(args.execute, args.dry_run)? {
        println!("Live mode selects the model with the greatest workerCount.");
        print_request(&request("<most-popular-live-model>", &args.prompt))?;
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("rust-event-driven"), Network::Fast).await?;
    let result = async {
        let models = client
            .projects
            .wait_for_models(Duration::from_secs(15))
            .await?;
        let model = models
            .iter()
            .max_by_key(|model| {
                model
                    .get("workerCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            })
            .ok_or_else(|| anyhow::anyhow!("the Supernet returned no available models"))?;
        let model_id = model.get("id").and_then(Value::as_str).unwrap_or_default();
        if model_id.is_empty() {
            bail!("the most popular model did not include an id");
        }
        println!(
            "Most popular model: {}",
            serde_json::to_string_pretty(model)?
        );
        let request = request(model_id, &args.prompt);
        let estimate = estimate_image(&client.projects, &request).await?;
        confirm_estimate(&estimate, args.yes)?;
        // Subscribe globally before submission so even an immediate first state is visible.
        let api_reporter = spawn_api_reporter(client.projects.clone());
        let project = client.projects.create(request).await?;
        let project_reporter = spawn_project_reporter(project.clone());
        // Events are observational; the project future supplies the terminal result.
        let urls = project.wait_for_completion(None).await?;
        project_reporter.abort();
        api_reporter.abort();
        println!("Project completed with {} result(s):", urls.len());
        for url in urls {
            println!("{url}");
        }
        Ok(())
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}
