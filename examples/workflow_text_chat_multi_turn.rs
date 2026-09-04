//! Interactive multi-turn socket-native chat with streamed responses.
//!
//! Each turn sends the system prompt and the accumulated user/assistant history to
//! a Supernet LLM worker, then renders response chunks as they arrive. The history
//! belongs to this process; it is not a durable hosted run. Commands include
//! `/clear`, `/history`, `/system <text>`, `/think`, and `/stats`; enter `exit` or
//! `quit` to stop. Failed turns are removed from history so a retry never inherits
//! a user message that the model did not successfully answer.
//!
//! Live use requires API-key or wallet-enabled username/password credentials and an
//! available LLM worker. `--help`, `--dry-run`, and the default invocation do not
//! load credentials. Pass `--execute` to enter the potentially paid session.
//! `--think` enables model reasoning; reasoning blocks remain hidden unless
//! `--show-thinking` is also supplied.
//!
//! The CLI reports a cost estimate before each turn, streams assistant text, and
//! tracks per-session prompt/completion tokens, elapsed time, and average
//! time-to-first-token. Model-catalog defaults are used for omitted sampling values,
//! with local fallbacks if the catalog is unavailable.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_text_chat_multi_turn -- --help
//! cargo run --example workflow_text_chat_multi_turn -- --dry-run
//! cargo run --example workflow_text_chat_multi_turn -- --execute --system "Reply like a pirate"
//! cargo run --example workflow_text_chat_multi_turn -- --execute --think --show-thinking --max-tokens 4096
//! ```

#[path = "workflow_text_chat_multi_turn/app.rs"]
mod app;
mod common;
#[path = "workflow_text_chat/shared/mod.rs"]
mod shared;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
