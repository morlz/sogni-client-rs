//! Benchmark text-to-image inference at each selected model's minimum, default,
//! and maximum supported step count.
//!
//! The example exercises model discovery, project submission, job completion,
//! result download, and a JSON timing report. Warm-up
//! projects are deliberately excluded from measured samples. The built-in tier
//! table is a reproducible snapshot for this benchmark, not an exhaustive model
//! catalog; live availability is still checked before work is submitted.
//!
//! `--help` and the default dry run need no credentials. Live runs require
//! `SOGNI_API_KEY`, or username/password with the crate's `wallet` feature, and
//! `--execute`; the total paid render count is confirmed unless `--yes` is
//! supplied. Results and reports are written below `examples/output/benchmark`
//! by default, while `--no-download` keeps timing/report generation only.
//!
//! ```text
//! cargo run --example benchmark_text_to_image -- --help
//! cargo run --example benchmark_text_to_image -- --models z-turbo --runs 2 --dry-run
//! cargo run --example benchmark_text_to_image -- --models z-turbo,qwen-2512-lightning --runs 3 --execute
//! cargo run --example benchmark_text_to_image -- --models z-turbo --runs 3 --execute --yes --no-download
//! ```

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
