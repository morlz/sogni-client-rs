//! Batch image-to-video generation with one configuration.
//!
//! The runner scans a folder for JPEG, PNG, and WebP images, sorts them, and
//! processes them sequentially. Output files inherit the input stem, and an
//! existing `<stem>.mp4` is skipped unless `--no-skip-existing` is supplied.
//! Dimensions default to the first image and are aligned to the selected model's
//! grid before each request is built.
//!
//! WAN 2.2 generates at 16 fps internally. Its `--fps 32` mode interpolates the
//! generated frames, so frame calculation still uses 16 fps. LTX 2.3/2.5 uses
//! the requested 1-60 fps directly and snaps frame counts to the model grid.
//! Video generation always uses the `fast` network.
//!
//! # Safety and credentials
//!
//! `--help` and `--dry-run` do not authenticate, upload media, or submit paid
//! work. Live generation requires Sogni credentials, `--execute`, and a cost
//! confirmation; add `--yes` only in an already-approved automation.
//!
//! # Usage
//!
//! ```text
//! cargo run --example workflow_batch_i2v -- --help
//! cargo run --example workflow_batch_i2v -- "camera pans left" --folder ./images --dry-run
//! cargo run --example workflow_batch_i2v -- "zoom in" --folder ./images --execute
//! cargo run --example workflow_batch_i2v -- --model ltx25-22b-int8_i2v_distilled --fps 24 --duration 5 --folder ./images --dry-run
//! ```
//!
//! Completed videos are downloaded under `--output` (default `output/`). A
//! per-video estimate is displayed before a live batch is submitted.

#[path = "workflow_batch_i2v/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
