//! Generate music with ACE-Step 1.5 audio projects.
//!
//! The example accepts a musical brief plus lyrics, duration, BPM, key/scale,
//! time signature, language, composer controls, sampler/scheduler, and output
//! format. XL Turbo is the default fast model and does not use CFG guidance;
//! SFT variants use CFG and allow a larger step range. The non-XL model IDs are
//! retained as legacy-compatible choices, not as a complete live catalog.
//!
//! Durations are 10-600 seconds, BPM is 30-300, supported time signatures are
//! 2, 3, 4, and 6, and output can be MP3, WAV, or FLAC. `--lyrics-file` reads
//! lyrics from disk and conflicts with inline `--lyrics`.
//!
//! # Safety and credentials
//!
//! `--help` and `--dry-run` validate and print the request without credentials
//! or paid generation. Live work requires credentials and explicit `--execute`;
//! the audio estimate is displayed for confirmation before submission.
//!
//! # Usage
//!
//! ```text
//! cargo run --example workflow_text_to_music -- --help
//! cargo run --example workflow_text_to_music -- "upbeat electronic dance music" --duration 30 --dry-run
//! cargo run --example workflow_text_to_music -- "jazz ballad" --duration 60 --execute
//! cargo run --example workflow_text_to_music -- "rock anthem" --model ace_step_1.5_xl_sft --lyrics-file lyrics.txt --dry-run
//! ```
//!
//! Completed tracks are downloaded beneath `--output` in the selected format.

#[path = "workflow_text_to_music/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
