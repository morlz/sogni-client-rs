//! Animate a first image, optionally converging on an end image.
//!
//! The example supports WAN 2.2 and the current LTX image-to-video families.
//! WAN generates at a fixed internal 16 fps; selecting 32 fps is post-render
//! interpolation and does not double generated inference frames. LTX uses the
//! requested 1-60 fps directly and follows its native frame grid.
//!
//! An `--end-image` supplies the final keyframe. `--transition` is an LTX-only
//! morph workflow: it requires both endpoints, attaches the transition LoRA,
//! and adds its `zhuanchang` trigger when absent. First/last-frame strengths are
//! independent values from 0 through 1. Speaker identity audio is also limited
//! to LTX models.
//!
//! # Safety and credentials
//!
//! `--help` and `--dry-run` are credential-free and perform no upload or paid
//! generation. A live request requires credentials and explicit `--execute`;
//! the runner shows the estimate and asks for confirmation before submission.
//!
//! # Usage
//!
//! ```text
//! cargo run --example workflow_image_to_video -- --help
//! cargo run --example workflow_image_to_video -- "zoom in" --image input.jpg --dry-run
//! cargo run --example workflow_image_to_video -- --image first.jpg --end-image last.jpg --dry-run
//! cargo run --example workflow_image_to_video -- "a woman morphs into a fox" --image woman.jpg --end-image fox.jpg --transition --model ltx23-22b-fp8_i2v_distilled --execute
//! ```
//!
//! Results are downloaded as MP4 files under `--output` (default `output/`).
//! The optional 10Eros model's author examples are at
//! <https://civitai.red/models/2447875/ltx23-10eros>; availability and accepted
//! model IDs should still be checked against the live model catalog.

#[path = "workflow_image_to_video/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
