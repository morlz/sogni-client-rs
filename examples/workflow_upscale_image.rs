//! Deterministically upscale an image with RTX VSR Pro while preserving aspect
//! ratio and respecting the service's image-size contract.
//!
//! The workflow needs no creative prompt: the source image is uploaded as the
//! starting image and a fixed one-step recipe preserves its content, identity,
//! composition, and colors while increasing resolution. Choose an
//! integer factor or an exact longest edge; the explicit longest edge wins.
//! Computed dimensions preserve aspect ratio, align to multiples of eight, keep
//! both edges at least 512 pixels, and cap the longest edge at 15,360 pixels.
//!
//! `--help` is credential-free. Dry runs validate/read the input image and print
//! the resolved request without connecting. Paid upscaling requires credentials,
//! `--execute`, and cost confirmation unless `--yes` is passed. PNG output is
//! downloaded to `examples/output` by default.
//!
//! ```text
//! cargo run --example workflow_upscale_image -- --help
//! cargo run --example workflow_upscale_image -- --image input.png --scale 2 --dry-run
//! cargo run --example workflow_upscale_image -- --image input.png --target 4096 --execute
//! cargo run --example workflow_upscale_image -- --image input.png --scale 4 --output examples/output/upscaled --execute --yes
//! ```

mod common;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use sogni_client::{AssetRole, MediaSource, Network, ProjectRequest};

use common::{
    auth::{close, connect, unique_app_id},
    cli::{
        BillingMode, TokenType, execution_requested, explain_dry_run, resolve_billing_mode,
        resolve_token_type,
    },
    files::{choose_file, image_dimensions},
    workflow::{estimate_and_confirm, print_request, require_model, run_image_project},
};

const MODEL_ID: &str = "rtx_vsr_pro";
const MIN_EDGE: u32 = 512;
const MAX_EDGE: u32 = 15_360;
const DIMENSION_STEP: u32 = 8;

#[derive(Debug, Parser)]
#[command(about = "Deterministically upscale an image with RTX VSR, up to 16K")]
struct Args {
    #[arg(long)]
    image: Option<PathBuf>,
    /// Relative enlargement; must be 2, 3, or 4. Ignored with --target.
    #[arg(long)]
    scale: Option<u32>,
    /// Exact longest output edge, from 512 through 15360 pixels.
    #[arg(long)]
    target: Option<u32>,
    #[arg(long, default_value = "examples/output")]
    output: PathBuf,
    #[arg(long)]
    no_interactive: bool,
    #[arg(long, alias = "billing", value_enum)]
    billing_mode: Option<BillingMode>,
    #[arg(long, value_enum)]
    token_type: Option<TokenType>,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

fn resolve_dimensions(source: (u32, u32), scale: f64) -> Result<(u32, u32)> {
    let (source_width, source_height) = source;
    let longest = source_width.max(source_height);
    if source_width == 0 || source_height == 0 {
        bail!("the source image has invalid dimensions");
    }
    if longest >= MAX_EDGE {
        bail!("the source is already at or above the {MAX_EDGE}px RTX VSR limit");
    }
    if !scale.is_finite() || scale <= 1.0 {
        bail!("the upscaled output must be larger than the source");
    }
    // Clamp before aligning down so neither output edge can cross the service ceiling.
    let scale = scale.min(f64::from(MAX_EDGE) / f64::from(longest));
    let align = |value: f64| (value as u32 / DIMENSION_STEP) * DIMENSION_STEP;
    let output = (
        align(f64::from(source_width) * scale),
        align(f64::from(source_height) * scale),
    );
    if output.0 < MIN_EDGE || output.1 < MIN_EDGE {
        bail!(
            "the output would be {}x{}, but each edge must be at least {MIN_EDGE}px",
            output.0,
            output.1
        );
    }
    Ok(output)
}

fn request(
    image: &std::path::Path,
    dimensions: (u32, u32),
    token: TokenType,
    billing: BillingMode,
) -> ProjectRequest {
    // RTX VSR is a deterministic transform; creative prompt fields stay deliberately empty.
    ProjectRequest::image(MODEL_ID, "")
        .network(Network::Fast)
        .number_of_media(1)
        .steps(1)
        .dimensions(dimensions.0, dimensions.1)
        .asset(
            AssetRole::StartingImage,
            MediaSource::Path(image.to_owned()),
        )
        .param("startingImageStrength", 1)
        .param("negativePrompt", "")
        .param("stylePrompt", "")
        .param("numberOfPreviews", 0)
        .param("tokenType", token.as_str())
        .param("billingMode", billing.as_str())
        .param("sizePreset", "custom")
        .param("outputFormat", "png")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.scale.is_some_and(|scale| !(2..=4).contains(&scale)) {
        bail!("--scale must be 2, 3, or 4; use --target for another size");
    }
    if args
        .target
        .is_some_and(|target| !(MIN_EDGE..=MAX_EDGE).contains(&target))
    {
        bail!("--target must be between {MIN_EDGE} and {MAX_EDGE}");
    }
    let interactive = !args.no_interactive;
    let image = choose_file(args.image.clone(), "source image", interactive)?;
    let source = image_dimensions(&image)?;
    let longest = source.0.max(source.1);
    // An exact target is more specific and therefore takes precedence over --scale.
    let scale = args
        .target
        .map(|target| f64::from(target) / f64::from(longest))
        .or(args.scale.map(f64::from))
        .unwrap_or_else(|| f64::from(MAX_EDGE) / f64::from(longest));
    let dimensions = resolve_dimensions(source, scale)?;
    let token = resolve_token_type(args.token_type, interactive && args.execute)?;
    let billing = resolve_billing_mode(args.billing_mode)?;
    let request = request(&image, dimensions, token, billing);
    println!(
        "RTX VSR: {}x{} -> {}x{} ({:.2}x effective)",
        source.0,
        source.1,
        dimensions.0,
        dimensions.1,
        f64::from(dimensions.0) / f64::from(source.0)
    );
    print_request(&request)?;
    if !execution_requested(args.execute, args.dry_run)? {
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("sogni-workflow-upscale"), Network::Fast).await?;
    let result = async {
        require_model(&client.projects, MODEL_ID).await?;
        estimate_and_confirm(&client.projects, &request, args.yes).await?;
        let prefix = format!("rtx-vsr-{}x{}", dimensions.0, dimensions.1);
        run_image_project(&client.projects, request, &args.output, &prefix, "png").await?;
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
    fn dimensions_preserve_ratio_align_and_cap() {
        assert_eq!(resolve_dimensions((800, 600), 2.0).unwrap(), (1600, 1200));
        assert_eq!(
            resolve_dimensions((8000, 4000), 4.0).unwrap(),
            (15360, 7680)
        );
        assert!(resolve_dimensions((120, 120), 2.0).is_err());
    }
}
