//! Socket-native chat routed into core Sogni media-composition pipelines.
//!
//! This single-turn example lets an LLM decide whether a request is ordinary
//! conversation or a request for an image, video, or music. It deliberately
//! demonstrates only the three core text-to-media tools. The SDK's broader public
//! Sogni tool surface also contains asset-backed operations such as `edit_image`,
//! `sound_to_video`, and `video_to_video`; those need explicit media inputs and are
//! outside this conversational example.
//!
//! # Six-stage pipeline
//!
//! 1. The routing LLM receives the user message and core intent schemas with
//!    `tool_choice: "auto"`.
//! 2. For a media request it emits a tool call containing the user's raw intent;
//!    otherwise its streamed text is the final conversational response.
//! 3. The client validates that call as a known Sogni intent and routes it to the
//!    image, video, or song composer.
//! 4. A specialized LLM call prompt-engineers a structured specification using one
//!    schema and `tool_choice: "required"`. Tool calling is the output channel so
//!    structured arguments are retained reliably; thinking is disabled for this
//!    composition call even when routing used thinking.
//! 5. The client estimates and confirms cost, creates a Projects API job, follows
//!    progress to completion, and downloads every result beneath `--output-dir`.
//! 6. It appends the original assistant tool-call message followed by matching tool
//!    results, then asks the routing LLM for a concise natural-language summary.
//!
//! Routing thinking is enabled by default; `--no-think` disables it and
//! `--show-thinking` exposes otherwise-hidden reasoning blocks. The example asks
//! for confirmation before the routing call, before each specialized composition
//! call, and before media generation (or before continuing without an estimate).
//! `--yes` accepts all of those prompts. Defaults are Z-Image Turbo, LTX-2.3, and
//! ACE-Step 1.5 XL Turbo, and can be overridden independently.
//!
//! Live use requires API-key or wallet-enabled username/password credentials plus
//! available LLM and matching media workers. `--help`, `--dry-run`, and the default
//! invocation are credential-free. Only `--execute` enables potentially paid LLM
//! and generation calls; a real prompt is required in that mode. Generated JPG,
//! MP4, or MP3 files are printed after download, followed by the model's summary.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_text_chat_sogni_tools -- --dry-run "Create an image of a cyberpunk city"
//! cargo run --example workflow_text_chat_sogni_tools -- --execute "Compose a jazz song about rain"
//! cargo run --example workflow_text_chat_sogni_tools -- --execute --duration 5 --aspect-ratio landscape "Generate a video of ocean waves"
//! cargo run --example workflow_text_chat_sogni_tools -- --execute --quantity 2 --yes "Create two botanical poster images"
//! ```

#[path = "workflow_text_chat_sogni_tools/app.rs"]
mod app;
mod common;
#[path = "workflow_text_chat_sogni_tools/composition.rs"]
mod composition;
#[path = "workflow_text_chat_sogni_tools/generation.rs"]
mod generation;
#[path = "workflow_text_chat_sogni_tools/schemas.rs"]
mod schemas;
#[path = "workflow_text_chat/shared/mod.rs"]
mod shared;
#[path = "workflow_text_chat_sogni_tools/types.rs"]
mod types;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
