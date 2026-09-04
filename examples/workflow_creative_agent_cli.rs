//! Interactive hosted creative-agent CLI with local Markdown context.
//!
//! This is a terminal client for the OpenAI-compatible hosted REST completion
//! endpoint, not socket-native worker chat. It keeps multi-turn history, builds a
//! bounded system context from customer-authored Markdown files, and asks the
//! hosted service to inject either the creative-tools family or the broader
//! creative-agent family. Slash commands can reload context, change billing and
//! tool modes, inspect the last response, clear history, or save a Markdown
//! transcript.
//!
//! Hosted chat requires `SOGNI_API_KEY` in the environment, `examples/.env`, or
//! `.env`; username/password authentication is not supported by this REST-only
//! example. Automatic context discovery looks for conventional Markdown files and
//! `.sogni` globs under `--workspace`. It skips build/VCS directories and symlinks,
//! and applies both per-file and total character budgets. The model sees only the
//! loaded Markdown and conversation messages, never arbitrary workspace files.
//!
//! `--help` and the default preview are credential-free. `--submit` is the live,
//! potentially paid network gate. Separately, `--no-execute` lets the server inject
//! tools without executing them; `--execute` restores that tool-execution setting
//! but does not replace `--submit`. Dry-run output redacts Markdown contents while
//! showing how many files and characters would be included.
//!
//! Responses show assistant text, tool calls/results, creative workflows, usage,
//! and discovered result URLs; `--json` prints the hosted payload. Interactive
//! state lasts for the process, while `/save [path]` writes a Markdown transcript
//! beneath `--output-dir` by default.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_creative_agent_cli -- --help
//! cargo run --example workflow_creative_agent_cli -- --dry-run "Create an image of a glass apple"
//! cargo run --example workflow_creative_agent_cli -- --submit --context ./artist-style.md --subscription
//! cargo run --example workflow_creative_agent_cli -- --submit --no-execute "Draft a product-video plan"
//! ```

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
