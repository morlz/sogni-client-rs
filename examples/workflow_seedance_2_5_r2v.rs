//! Seedance 2.5 reference, edit, and extend operations.
//!
//! The same intent can be expressed through two layers:
//!
//! - `--layer direct` creates a Projects API video job and sends
//!   `seedanceTaskType=reference|edit|extend` for Seedance 2.5.
//! - `--layer creative-agent` creates a durable workflow using
//!   `generate_video`, `video_to_video`, or `extend_video` respectively.
//!
//! `reference` accepts loose image/video/audio conditioning. `edit` and
//! `extend` require `@Video1`; direct edit additionally requires `--duration`
//! equal to that source video's duration, while extend duration is the new
//! continuation length. Attachments are independently numbered as `@ImageN`,
//! `@VideoN`, and `@AudioN` in command order.
//!
//! Seedance 2.5 is fixed at 24 fps, supports 4-30 second output at 480p or 720p,
//! accepts 30 images, 10 videos, 10 audios, and 50 files total, and permits an
//! audio-only reference job. The 2.0 compatibility family is limited to 4-15
//! seconds and 9/3/3/12 media; audio requires accompanying visual media and the
//! direct request omits the 2.5-only task field. The retired 2.0 Fast ID is
//! recognized only for direct compatibility and has no Creative Agent selector.
//!
//! # Safety and credentials
//!
//! `--help` and `--dry-run` validate and print the request without credentials,
//! media upload, or generation. Local media is uploaded only after explicit
//! `--execute`; hosted input must use HTTPS. Live execution requires Sogni
//! credentials and confirmation of the paid request. `--watch` streams durable
//! workflow events until a terminal state.
//!
//! # Usage
//!
//! ```text
//! cargo run --example workflow_seedance_2_5_r2v -- --help
//! cargo run --example workflow_seedance_2_5_r2v -- "Use @Image1 for the product and @Video1 for camera motion" --task-type reference --image product.jpg --video motion.mp4 --dry-run
//! cargo run --example workflow_seedance_2_5_r2v -- "Edit @Video1 while preserving subject and timing" --task-type edit --video source.mp4 --duration 5 --execute
//! cargo run --example workflow_seedance_2_5_r2v -- "Extend @Video1 after its ending" --task-type extend --video source.mp4 --layer creative-agent --watch --execute
//! ```
//!
//! Direct results are downloaded as MP4 files beneath `--output`. Creative Agent
//! mode prints the durable workflow record and, with `--watch`, its event stream.

#[path = "workflow_seedance_2_5_r2v/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
