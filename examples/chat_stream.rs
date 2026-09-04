//! Stream an LLM completion from Sogni's socket-native chat API.
//!
//! The example sends one user message through `chat.completions`, prints content
//! chunks as they arrive, and requires a terminal result before reporting
//! success. The default model is the shared chat example default; callers can
//! override it, but should rely on the live catalog for availability and limits.
//!
//! `--help` and the default request preview are credential-free. A live chat is
//! billable and therefore requires Sogni credentials plus `--execute`. This
//! example writes streamed text to stdout and does not create output files.
//!
//! ```text
//! cargo run --example chat_stream -- --help
//! cargo run --example chat_stream -- "Pitch three surreal album covers" --dry-run
//! cargo run --example chat_stream -- "Explain chiaroscuro in one paragraph" --execute
//! cargo run --example chat_stream -- "Describe this composition" --model qwen3.6-35b-a3b-gguf-iq4xs --execute
//! ```

mod common;
#[path = "workflow_text_chat/shared/mod.rs"]
mod shared;

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::json;
use sogni_client::Network;

#[derive(Parser)]
#[command(about = "Stream an LLM response over the Sogni socket transport")]
struct Args {
    #[arg(default_value = "Pitch three surreal album covers.")]
    prompt: String,
    #[arg(long, default_value = shared::runtime::DEFAULT_MODEL)]
    model: String,
    /// Perform the paid network request. Without this flag the example is a dry run.
    #[arg(long)]
    execute: bool,
    #[arg(long, conflicts_with = "execute")]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let request = json!({
        "model": args.model,
        "messages": [{"role": "user", "content": args.prompt}],
        "stream": true,
    });
    if !args.execute || args.dry_run {
        println!("Dry run; pass --execute to submit this paid request.\n{request:#}");
        return Ok(());
    }
    if args.prompt.trim().is_empty() {
        bail!("prompt must not be empty");
    }
    let client = common::auth::connect(
        common::auth::unique_app_id("sogni-client-rs-chat-stream"),
        Network::Fast,
    )
    .await?;
    print!("Assistant: ");
    let outcome = shared::stream_response(&client, &request, true).await;
    println!();
    let close = common::auth::close(&client).await;
    outcome?;
    close
}
