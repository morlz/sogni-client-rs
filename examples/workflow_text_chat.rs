//! One-shot, non-streaming chat through the socket-native Sogni LLM protocol.
//!
//! The request is sent to an online Supernet LLM worker and resolves only when the
//! complete [`sogni_client::ChatCompletion`] arrives. This differs from hosted REST
//! chat, which can inject and execute server-side Sogni tools, and from durable chat
//! runs, whose state survives a client disconnect. Omit the positional prompt with
//! `--execute` to read one prompt interactively from standard input.
//!
//! Live use requires `SOGNI_API_KEY`, or `SOGNI_USERNAME` plus `SOGNI_PASSWORD`
//! when the default `wallet` feature is enabled, and an available LLM worker. The
//! environment, `examples/.env`, and `.env` are checked in that order. `--help`,
//! `--dry-run`, and the default invocation are credential-free; only `--execute`
//! submits the potentially paid request.
//!
//! Explicit sampling/token flags take precedence. Missing values are filled from
//! the selected model's advertised defaults when the socket catalog is available,
//! then from conservative local fallbacks. Before submission the example attempts
//! a cost estimate; the server remains authoritative. Output includes assistant
//! content, worker, finish reason, timing, token usage, throughput, and cost.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_text_chat -- --help
//! cargo run --example workflow_text_chat -- --dry-run "What is the meaning of life?"
//! cargo run --example workflow_text_chat -- --execute --max-tokens 100 --temperature 0.9 "Write a haiku"
//! cargo run --example workflow_text_chat -- --execute --think --task-profile reasoning "Explain this step by step"
//! ```

#[path = "workflow_text_chat/app.rs"]
mod app;
mod common;
#[path = "workflow_text_chat/shared/mod.rs"]
mod shared;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
