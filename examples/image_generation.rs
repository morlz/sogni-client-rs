mod common;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use sogni_client::{Network, ProjectRequest};

use common::{
    auth::{close, connect, unique_app_id},
    cli::{execution_requested, explain_dry_run},
    workflow::{estimate_and_confirm, print_request, require_model},
};

#[derive(Debug, Parser)]
#[command(about = "Generate one or more images with the Sogni Supernet")]
struct Args {
    #[arg(
        long,
        default_value = "A glass greenhouse drifting above Singapore at dawn"
    )]
    prompt: String,
    #[arg(long, default_value = "z_image_turbo_bf16")]
    model: String,
    #[arg(long, default_value_t = 1024)]
    width: u32,
    #[arg(long, default_value_t = 1024)]
    height: u32,
    #[arg(long, default_value_t = 8)]
    steps: u32,
    #[arg(long, default_value_t = 1)]
    count: u32,
    #[arg(long, default_value = "output/images")]
    output: PathBuf,
    /// Download results in addition to printing their temporary URLs.
    #[arg(long)]
    download: bool,
    /// Perform the paid network request. Without this flag the example is a dry run.
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    /// Accept the displayed cost estimate without an interactive confirmation.
    #[arg(long)]
    yes: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.count == 0 {
        bail!("--count must be at least 1");
    }
    let request = ProjectRequest::image(&args.model, &args.prompt)
        .dimensions(args.width, args.height)
        .steps(args.steps)
        .number_of_media(args.count)
        .network(Network::Fast)
        .param("negativePrompt", "text, watermark")
        .param("tokenType", "spark");
    print_request(&request)?;
    if !execution_requested(args.execute, args.dry_run)? {
        explain_dry_run();
        return Ok(());
    }

    let client = connect(unique_app_id("rust-image-generation"), Network::Fast).await?;
    let result = async {
        require_model(&client.projects, &args.model).await?;
        estimate_and_confirm(&client.projects, &request, args.yes).await?;
        let project = client.projects.create(request).await?;
        let urls = common::progress::wait_with_progress(&project).await?;
        for url in &urls {
            println!("{url}");
        }
        if args.download {
            common::files::download_results(&urls, &args.output, "image", "jpg").await?;
        }
        Ok(())
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}
