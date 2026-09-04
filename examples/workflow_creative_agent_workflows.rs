//! Durable creative-agent workflow lifecycle over Sogni's REST API.
//!
//! Unlike socket-native chat, this example creates and manages server-persisted,
//! asynchronous workflows. The default `start` action submits an explicit two-step
//! plan: generate a keyframe, then pass that image artifact to a video step. The
//! remaining actions inspect or control an existing workflow without rebuilding the
//! plan locally.
//!
//! The REST surface requires API-key authentication through `SOGNI_API_KEY`. Help
//! and dry runs are credential-free; every live operation, including reads and SSE
//! streaming, requires `--execute`. Starting, resuming, and reseeding can add
//! `--watch` to consume events immediately. Standalone `stream` supports `--after`
//! and the SSE `--last-event-id` cursor so a disconnected observer can continue
//! from a known position. Because workflow state is durable, the work itself is not
//! tied to this process or to the lifetime of an SSE connection.
//!
//! Output includes the workflow id and status, any artifacts returned by snapshots,
//! and event id/type/data while streaming. `cancel` requests server-side
//! cancellation; `resume` releases a workflow paused in `waiting_for_user`;
//! `reseed` clones a completed run and reports its source and reseeded steps.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_creative_agent_workflows -- --dry-run "A chrome monorail over neon gardens"
//! cargo run --example workflow_creative_agent_workflows -- --execute --watch "A cinematic robot portrait"
//! cargo run --example workflow_creative_agent_workflows -- --execute --stream workflow_123 --last-event-id 42
//! cargo run --example workflow_creative_agent_workflows -- --execute --cancel workflow_123
//! ```

#[path = "workflow_creative_agent_workflows/app.rs"]
mod app;
#[path = "workflow_creative_agent_workflows/cli.rs"]
mod cli;
mod common;
#[path = "workflow_creative_agent_workflows/output.rs"]
mod output;
#[path = "workflow_creative_agent_workflows/request.rs"]
mod request;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
