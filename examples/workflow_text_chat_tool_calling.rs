//! Bounded socket-native custom function-calling loop.
//!
//! The LLM may request one or more caller-defined tools. This process appends the
//! assistant message containing those calls, executes each call locally in wire
//! order, appends one matching `tool` result per call id, and asks the model again
//! for a natural-language answer. The loop stops when no calls are returned or
//! fails after `--max-rounds` (bounded to at most 20). These are custom client-side
//! functions, not hosted/Sogni tools; the example never executes a server tool
//! locally.
//!
//! Built-ins are arithmetic, unit conversion, current time, and weather. Arithmetic
//! and conversion are local. Weather calls `https://wttr.in`, while time calls
//! `https://worldtimeapi.org`, so those tools need outbound internet even after the
//! socket chat connects. Tool argument, service, and unknown-tool failures become
//! structured error results that the model can explain rather than corrupting call
//! ordering.
//!
//! Live chat requires API-key or wallet-enabled username/password credentials and
//! an online LLM worker. `--help`, `--dry-run`, and the default invocation are
//! credential-free and do not execute any tools. Pass `--execute` for the
//! potentially paid chat/tool loop. Output streams each assistant round, previews
//! each tool result, and reports per-round plus total timing, usage, and cost.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_text_chat_tool_calling -- --dry-run "What's the weather in Austin?"
//! cargo run --example workflow_text_chat_tool_calling -- --execute "What time is it in Tokyo and London?"
//! cargo run --example workflow_text_chat_tool_calling -- --execute "Convert 72 F to Celsius"
//! cargo run --example workflow_text_chat_tool_calling -- --execute --max-rounds 3 "What's 15% of 249.99?"
//! ```

#[path = "workflow_text_chat_tool_calling/app.rs"]
mod app;
mod common;
#[path = "workflow_text_chat/shared/mod.rs"]
mod shared;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
