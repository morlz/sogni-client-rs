#[path = "benchmark_text_to_image/app.rs"]
mod app;
mod common;
#[path = "benchmark_text_to_image/config.rs"]
mod config;
#[path = "benchmark_text_to_image/report.rs"]
mod report;
#[path = "benchmark_text_to_image/run.rs"]
mod run;
#[path = "benchmark_text_to_image/types.rs"]
mod types;

use anyhow::Result;
use clap::Parser as _;

#[tokio::main]
async fn main() -> Result<()> {
    app::run(config::Args::parse()).await
}
