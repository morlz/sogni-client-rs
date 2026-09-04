//! Generate video from a text prompt with WAN or LTX models.
//!
//! WAN 2.2 always generates at 16 fps internally. `--fps 16` returns those
//! frames directly, while `--fps 32` requests post-render interpolation; its
//! inference-frame calculation is therefore based on 16 fps in both cases.
//! LTX 2.3/2.5 generates at the requested 1-60 fps and snaps the result to its
//! native frame grid. WAN accepts motion `shift`; LTX does not. Optional speaker
//! identity audio is supported only by compatible LTX text-to-video models.
//!
//! # Safety and credentials
//!
//! Video generation uses the `fast` network. `--help` and `--dry-run` are
//! credential-free and do not submit paid work. Live execution requires Sogni
//! credentials and explicit `--execute`; the cost estimate is confirmed before
//! the project is created.
//!
//! # Usage
//!
//! ```text
//! cargo run --example workflow_text_to_video -- --help
//! cargo run --example workflow_text_to_video -- "A futuristic city" --dry-run
//! cargo run --example workflow_text_to_video -- "Dancing robots" --fps 32 --execute
//! cargo run --example workflow_text_to_video -- "Ocean waves" --model ltx25-22b-int8_t2v_distilled --fps 24 --duration 5 --dry-run
//! ```
//!
//! Results are streamed with progress and downloaded as MP4 files beneath
//! `--output` (default `output/`). Treat listed model IDs as examples and query
//! the live catalog before depending on availability.

#[path = "workflow_text_to_video/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
