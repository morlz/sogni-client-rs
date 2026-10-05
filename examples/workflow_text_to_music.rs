//! Generate music with MiniMax Music 3 or ACE-Step 1.5 audio projects.
//!
//! The example accepts a musical brief plus lyrics, duration, BPM, key/scale,
//! time signature, language, composer controls, sampler/scheduler, and output
//! format. ACE-Step XL Turbo is a fast draft model with no CFG guidance;
//! SFT variants use CFG and allow a larger step range. The non-XL model IDs are
//! retained as legacy-compatible choices, not as a complete live catalog.
//!
//! MiniMax Music 3 is the default: 10-300 seconds (default 60), 10-100 steps
//! (default 30), guidance 1-5 (default 1.7), and prompt strength 0-10 (default
//! 1.7). Duration is a ceiling; a track can end earlier at a musical resolution.
//! Tempo, key, meter, and language flags become prompt directions for Music 3.
//! Empty Music 3 lyrics get plain section tags for instrumental structure.
//!
//! ACE-Step is available by canonical `--model`. A model-less duration above
//! 300 seconds, `--shift`, `--no-composer-mode`, or `--creativity` selects XL
//! Turbo. ACE-Step supports 10-600 seconds (default 30), BPM 30-300, and time
//! signatures 2, 3, 4, and 6. Explicit Music 3 requests reject ACE-only controls
//! and durations over 300. Output can be MP3, WAV, or FLAC. `--lyrics-file` reads
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
