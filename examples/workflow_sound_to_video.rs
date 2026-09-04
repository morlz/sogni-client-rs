//! Generate video driven by an audio track.
//!
//! Three public workflow shapes are demonstrated:
//!
//! - WAN 2.2 S2V uses a reference image and audio for synchronized motion.
//! - LTX IA2V uses both an image and audio.
//! - LTX A2V uses audio alone and rejects `--image`.
//!
//! WAN generates at 16 fps internally; 32 fps is interpolated output. LTX uses
//! the requested 1-60 fps directly and snaps to its frame grid. If `--duration`
//! is omitted during live execution, the runner probes the audio duration.
//! Current workers consume an M4A driving asset, so non-M4A input is converted
//! to AAC/M4A with FFmpeg and the temporary file is removed automatically.
//!
//! # Prerequisites and safety
//!
//! Video generation requires the `fast` network. Install FFmpeg when supplying
//! MP3/WAV or another format that needs conversion. `--help` and `--dry-run`
//! require no credentials and submit nothing. Live work requires credentials,
//! `--execute`, and confirmation of the server-provided estimate.
//!
//! # Usage
//!
//! ```text
//! cargo run --example workflow_sound_to_video -- --help
//! cargo run --example workflow_sound_to_video -- --image person.jpg --audio speech.m4a --dry-run
//! cargo run --example workflow_sound_to_video -- "A person speaking" --image face.jpg --audio voice.mp3 --execute
//! cargo run --example workflow_sound_to_video -- "A music visualizer" --audio music.m4a --model ltx23-a2v-distilled --dry-run
//! ```
//!
//! Completed MP4 files are downloaded under `--output` (default `output/`).

#[path = "workflow_sound_to_video/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
