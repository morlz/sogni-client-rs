//! Generate text-to-image or supported image-to-image projects across the Sogni
//! image model families represented by this example.
//!
//! The CLI applies model-specific dimensions, step/guidance ranges, sampler,
//! scheduler, and negative-prompt defaults, while still validating overrides.
//! An optional starting image and strength are included only for compatible
//! models. LoRA ids and strengths are an ordered positional pair. The resolved
//! request seed is carried into output names, and optional preview images plus
//! final media are downloaded to the configured output directory.
//!
//! `--help` is credential-free. Non-interactive dry runs build and print the
//! request without credentials; local starting-image paths are validated. Paid
//! generation requires credentials, `--execute`, and estimate confirmation
//! unless `--yes` is supplied. The static choices are example presets rather
//! than a promise that every catalog model is online.
//!
//! ```text
//! cargo run --example workflow_text_to_image -- --help
//! cargo run --example workflow_text_to_image -- "A glass city at dawn" --model z-turbo --no-interactive --dry-run
//! cargo run --example workflow_text_to_image -- "Restyle this scene" --model qwen-2512-lightning --starting-image input.png --execute
//! cargo run --example workflow_text_to_image -- "Editorial portrait" --batch 4 --previews 2 --execute --yes
//! ```

#[path = "workflow_text_to_image/app.rs"]
mod app;
mod common;
#[path = "workflow_text_to_image/config.rs"]
mod config;

use anyhow::Result;
use clap::Parser as _;

#[tokio::main]
async fn main() -> Result<()> {
    app::run(config::Args::parse()).await
}
