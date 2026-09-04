//! Direct execution of a known synchronous hosted creative tool.
//!
//! This REST example calls `/v1/creative-agent/tools/execute` through
//! `SogniClient::chat`. It is appropriate when an application already knows the
//! exact composition or planning operation and arguments, so it can avoid an extra
//! LLM round whose only job would be choosing a tool. The allowlisted tools are
//! `enhance_prompt`, `compose_script`, `compose_lyrics`, `compose_instrumental`,
//! `compose_workflow`, and `compose_workflow_template`; arbitrary or media-project
//! execution is intentionally outside this direct surface.
//!
//! A live call requires `SOGNI_API_KEY` in the environment, `examples/.env`, or
//! `.env`. Username/password authentication is not available on this REST-only
//! path. `--help` and dry-run request rendering need no credentials, while
//! `--execute` is the explicit potentially paid network gate.
//!
//! The example prints the most useful returned message, prompt, script, lyrics, or
//! structure when available. `--json` prints the entire response for applications
//! that need the complete structured result.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_direct_creative_tool -- --dry-run "A portrait of a glass robot"
//! cargo run --example workflow_direct_creative_tool -- --execute --tool compose_script --destination-tool generate_video "Make a five-second LTX prompt"
//! cargo run --example workflow_direct_creative_tool -- --execute --tool compose_workflow "Plan a three-shot neon bakery teaser"
//! cargo run --example workflow_direct_creative_tool -- --execute --tool compose_lyrics --json "A synth-pop song about rain"
//! ```

#[path = "workflow_direct_creative_tool/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
