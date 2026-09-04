//! Transform a source video with LTX ControlNet or WAN Animate workflows.
//!
//! LTX supports `canny`, `pose`, `depth`, and `detailer` controls. Compatible
//! variants also support positional `outpaint` and masked `inpaint`. Pose may
//! take an image to supply appearance; inpaint requires `--mask`, and outpaint
//! requires `--outpaint-position`. WAN Animate Move and Animate Replace require
//! both `--video` and `--image`; normalized `--sam2-coords` applies only to
//! Animate Replace.
//!
//! During live execution the source is probed for dimensions, duration, and
//! frame rate. Explicit CLI values win, dimensions are aligned to the model
//! grid, and requested frames never bypass model validation. WAN generates at
//! 16 fps internally with optional 32 fps interpolation. LTX uses its requested
//! 1-60 fps directly. Compatible LTX workflows can also use identity audio.
//!
//! # Safety and credentials
//!
//! `--help` and `--dry-run` do not read/upload media or create paid work. Live
//! execution requires credentials, `--execute`, valid local inputs, and cost
//! confirmation.
//!
//! # Usage
//!
//! ```text
//! cargo run --example workflow_video_to_video -- --help
//! cargo run --example workflow_video_to_video -- --video source.mp4 --model ltx25-v2v-distilled --control-type canny --dry-run
//! cargo run --example workflow_video_to_video -- "glowing blue eyes" --video face.mp4 --control-type inpaint --mask mask.png --execute
//! cargo run --example workflow_video_to_video -- --image portrait.jpg --video motion.mp4 --model move-lightx2v --dry-run
//! ```
//!
//! Generated MP4 files are downloaded beneath `--output` (default `output/`).

#[path = "workflow_video_to_video/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
