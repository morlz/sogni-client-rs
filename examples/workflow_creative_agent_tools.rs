//! Hosted Sogni creative-tool injection through REST chat completions.
//!
//! This example sends one OpenAI-compatible `/v1/chat/completions` request. The
//! hosted API, rather than this process, injects the selected Sogni tool family and
//! may execute those tools server-side. `creative-tools` (also accepted as `rich`)
//! exposes the media/planning family; `creative-agent` adds deeper workflow and
//! control tools. This is distinct from socket-native custom function calling,
//! where the caller owns and executes every function.
//!
//! Set `SOGNI_API_KEY` in the environment, `examples/.env`, or `.env` for a live
//! request. This REST-only surface does not accept username/password credentials.
//! Help and the default body preview are credential-free. Only `--submit` performs
//! the potentially paid request. `--no-execute` still asks the server to inject
//! tool definitions but disables server-side Sogni tool execution.
//!
//! Normal output prints assistant content, returned tool-call names, and durable
//! creative-workflow ids/statuses when present. Use `--json` to inspect the full
//! response envelope.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_creative_agent_tools -- --help
//! cargo run --example workflow_creative_agent_tools -- --dry-run "Plan a four-shot red sneaker video"
//! cargo run --example workflow_creative_agent_tools -- --submit --tools creative-tools --no-execute "Write an orbit-video prompt"
//! cargo run --example workflow_creative_agent_tools -- --submit --tools creative-agent --json "Plan a neon bakery campaign"
//! ```

#[path = "workflow_creative_agent_tools/app.rs"]
mod app;
mod common;
#[path = "workflow_text_chat/shared/mod.rs"]
mod shared;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
