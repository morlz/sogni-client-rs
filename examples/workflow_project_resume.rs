//! Demonstrate durable project recovery after an intentional socket disconnect.
//!
//! Client A submits an image batch and disconnects after the first job event.
//! Client B reconnects with the same application id, consumes active/completed
//! recovery state, resumes progress, downloads all results, performs a final
//! clean sync, and classifies requested ids with `ProjectsApi::resolve_missing`.
//! Disconnecting the transport must not itself fail an in-flight project.
//!
//! `--help` and dry-run recovery-plan output need no credentials. The live path
//! creates paid media and requires credentials, `--execute`, and estimate
//! confirmation unless `--yes` is passed. Timeout flags bound only this example's
//! waits; they do not cancel server-side generation. Results go to `output/` by
//! default, and an actual failure path requests project cancellation.
//!
//! ```text
//! cargo run --example workflow_project_resume -- --help
//! cargo run --example workflow_project_resume -- --number 2 --dry-run
//! cargo run --example workflow_project_resume -- --number 3 --execute
//! cargo run --example workflow_project_resume -- --resolve-id PROJECT_ID --execute --yes
//! ```

#[path = "workflow_project_resume/app.rs"]
mod app;
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
