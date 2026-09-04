mod common;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use sogni_client::{AssetRole, MediaSource, Network, ProjectRequest};

use common::{
    auth::{close, connect, unique_app_id},
    cli::{execution_requested, explain_dry_run},
    files::require_file,
    workflow::{estimate_and_confirm, print_request, require_model},
};

#[derive(Debug, Parser)]
#[command(about = "Edit one or two reference images while preserving subject identity")]
struct Args {
    /// One or two reference images; put the base scene first.
    #[arg(required = true, num_args = 1..=2)]
    reference_images: Vec<PathBuf>,
    #[arg(long)]
    prompt: String,
    #[arg(long, default_value_t = 1)]
    count: u32,
    #[arg(long, default_value_t = 1024)]
    width: u32,
    #[arg(long, default_value_t = 1024)]
    height: u32,
    #[arg(long, default_value_t = 10)]
    steps: u32,
    #[arg(long, default_value = "output/krea-identity-edit")]
    output: PathBuf,
    /// Download results in addition to printing their temporary URLs.
    #[arg(long)]
    download: bool,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

fn build_request(args: &Args) -> ProjectRequest {
    let mut request = ProjectRequest::image("krea2_identity_edit_v1_2", &args.prompt)
        .dimensions(args.width, args.height)
        .steps(args.steps)
        .guidance(1.0)
        .number_of_media(args.count)
        .network(Network::Fast)
        .param("tokenType", "spark");
    for (index, path) in args.reference_images.iter().enumerate() {
        request = request.asset(
            AssetRole::ContextImage(u8::try_from(index + 1).expect("at most two images")),
            MediaSource::Path(path.clone()),
        );
    }
    request
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.prompt.trim().is_empty() {
        bail!("--prompt is required and cannot be empty");
    }
    if args.count == 0 {
        bail!("--count must be at least 1");
    }
    for path in &args.reference_images {
        require_file(path, "reference image")?;
    }
    let request = build_request(&args);
    print_request(&request)?;
    if !execution_requested(args.execute, args.dry_run)? {
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("rust-krea-identity-edit"), Network::Fast).await?;
    let result = async {
        require_model(&client.projects, "krea2_identity_edit_v1_2").await?;
        estimate_and_confirm(&client.projects, &request, args.yes).await?;
        let project = client.projects.create(request).await?;
        let urls = common::progress::wait_with_progress(&project).await?;
        for url in &urls {
            println!("{url}");
        }
        if args.download {
            common::files::download_results(&urls, &args.output, "identity-edit", "jpg").await?;
        }
        Ok(())
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}
