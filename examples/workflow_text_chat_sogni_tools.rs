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
