//! Submit an image project, await its terminal result, and download every image.
//!
//! This example presents the future/promise-style alternative to explicit event
//! handling. It discovers a currently staffed image model, estimates the batch,
//! creates one project, reports progress while awaiting completion, and saves
//! the returned media. Model discovery is dynamic, so no static model choice is
//! assumed to be permanently available.
//!
//! `--help` and dry-run request rendering need no credentials. Live generation
//! requires Sogni credentials and `--execute`, with cost confirmation unless
//! `--yes` is passed. Downloads are written to `images/` by default.
//!
//! ```text
//! cargo run --example promise_based -- --help
//! cargo run --example promise_based -- --prompt "A paper city at blue hour" --count 2 --dry-run
//! cargo run --example promise_based -- --prompt "A paper city at blue hour" --execute
//! cargo run --example promise_based -- --count 4 --output examples/output/promise --execute --yes
//! ```

mod common;

use std::{path::PathBuf, time::Duration};

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::Value;
use sogni_client::{Network, ProjectRequest};

use common::{
    auth::{close, connect, unique_app_id},
    cli::{confirm_estimate, execution_requested, explain_dry_run},
    files::download_results,
    progress::wait_with_progress,
    workflow::{estimate_image, print_request},
};

#[derive(Debug, Parser)]
#[command(about = "Promise-style image generation: submit, await, then download")]
struct Args {
    #[arg(long, default_value = "A cat wearing a hat")]
    prompt: String,
    #[arg(long, default_value_t = 4)]
    count: u32,
    #[arg(long, default_value = "images")]
    output: PathBuf,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

fn request(model_id: &str, args: &Args) -> ProjectRequest {
    ProjectRequest::image(model_id, &args.prompt)
        .steps(20)
        .guidance(7.5)
        .number_of_media(args.count)
        .network(Network::Fast)
        .param("negativePrompt", "malformation, bad anatomy, bad hands, missing fingers, cropped, low quality, bad quality, jpeg artifacts, watermark")
        .param("stylePrompt", "anime")
        .param("outputFormat", "jpg")
        .param("tokenType", "spark")
        .param("billingMode", std::env::var("SOGNI_BILLING_MODE").unwrap_or_else(|_| "auto".into()))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.count == 0 {
        bail!("--count must be at least 1");
    }
    if !execution_requested(args.execute, args.dry_run)? {
        println!("Live mode selects the model with the greatest workerCount.");
        print_request(&request("<most-popular-live-model>", &args))?;
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("rust-promise-image"), Network::Fast).await?;
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
            bail!("selected model did not include an id");
        }
        println!(
            "Using model: {}",
            model
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(model_id)
        );
        let request = request(model_id, &args);
        confirm_estimate(&estimate_image(&client.projects, &request).await?, args.yes)?;
        let project = client.projects.create(request).await?;
        let urls = wait_with_progress(&project).await?;
        download_results(&urls, &args.output, "image", "jpg").await?;
        Ok(())
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}
