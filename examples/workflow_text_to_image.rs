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
