#[path = "workflow_creative_agent_cli/app.rs"]
mod app;
mod common;
#[path = "workflow_creative_agent_cli/context.rs"]
mod context;
#[path = "workflow_creative_agent_cli/interactive.rs"]
mod interactive;
#[path = "workflow_creative_agent_cli/response.rs"]
mod response;
#[path = "workflow_creative_agent_cli/runner.rs"]
mod runner;
#[path = "workflow_creative_agent_cli/session.rs"]
mod session;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
