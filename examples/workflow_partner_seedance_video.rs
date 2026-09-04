//! Exercise partner-hosted Seedance through the API path for each media shape.
//!
//! Pure text-to-video defaults to hosted `/v1/chat/completions` with a forced
//! OpenAI-style Sogni tool call. Media-bearing I2V, IA2V, and V2V use a durable
//! Creative Agent workflow with explicit steps and retrievable HTTPS media.
//! Mode inference prefers video, then audio, then image inputs; an explicit
//! `--mode` or `--target` still wins and is validated against attached media.
//!
//! Seedance output is fixed at 24 fps and may include native audio. Seedance 2.0
//! supports 4-15 second clips; Mini is its 720p-oriented variant. Seedance 2.5
//! supports 4-30 seconds, first/last-frame conditioning, and output up to the
//! 720p tier. The 2.5 media limits are 30 images, 10 videos, 10 audios, and 50
//! assets total; the 2.0 family keeps 9/3/3/12. `--fast` is only a legacy alias
//! for `--mini`, not a separate current model.
//!
//! In prompts, `@Image1`, `@Video1`, and `@Audio1` are numbered independently by
//! modality and attachment order. Local media is uploaded only for execution;
//! already-hosted references must be HTTPS. Negative prompts are not supported,
//! so preservation and exclusion requirements should be written positively.
//!
//! # Safety and credentials
//!
//! `--help` and `--dry-run` are credential-free and do not upload or execute
//! tools. A live request requires Sogni credentials and explicit `--execute`.
//! Unless `--no-estimate` is chosen, the runner obtains an estimate and asks for
//! confirmation first. Use `--watch` to follow a durable workflow and
//! `--inspect-workflow` to fetch workflow records returned by hosted chat.
//!
//! # Usage
//!
//! ```text
//! cargo run --example workflow_partner_seedance_video -- --help
//! cargo run --example workflow_partner_seedance_video -- "A cinematic glass whale" --dry-run
//! cargo run --example workflow_partner_seedance_video -- "transition from day to night" --mode i2v --image day.jpg --end-image night.jpg --target workflow --dry-run
//! cargo run --example workflow_partner_seedance_video -- "a portrait sings on stage" --mode ia2v --image portrait.jpg --audio speech.m4a --target workflow --execute
//! ```
//!
//! Hosted responses are printed. Any result URLs present in the response are
//! downloaded as MP4 files beneath `--output` (default `output/`).

#[path = "workflow_partner_seedance_video/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
