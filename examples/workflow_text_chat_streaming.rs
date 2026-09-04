//! One-shot socket-native chat with token-by-token output.
//!
//! This example requests a streaming completion from a Supernet LLM worker, writes
//! content chunks immediately, and then reads the terminal completion for final
//! usage, cost, worker, and finish metadata. It is not the hosted REST endpoint and
//! does not create a durable server-side chat run.
//!
//! Live use requires API-key credentials or wallet-enabled username/password
//! credentials plus an online LLM worker. `--help`, `--dry-run`, and the default
//! invocation only render the request and do not load credentials. `--execute` is
//! required for the potentially paid request; without a positional prompt, the
//! live mode prompts once on standard input.
//!
//! Thinking is disabled by default. `--think` enables it, while
//! `--show-thinking` controls whether `<think>...</think>` blocks are displayed;
//! hidden blocks are filtered safely even when their tags span chunks. The summary
//! includes time-to-first-token and warns when thinking consumes the output budget.
//! Explicit tuning flags override model-catalog defaults and local fallbacks.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_text_chat_streaming -- --help
//! cargo run --example workflow_text_chat_streaming -- --dry-run "Tell me a story about a cat"
//! cargo run --example workflow_text_chat_streaming -- --execute --max-tokens 500 --temperature 1.0 "Write a poem"
//! cargo run --example workflow_text_chat_streaming -- --execute --think --show-thinking "Solve a logic puzzle"
//! ```

#[path = "workflow_text_chat_streaming/app.rs"]
mod app;
mod common;
#[path = "workflow_text_chat/shared/mod.rs"]
mod shared;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
