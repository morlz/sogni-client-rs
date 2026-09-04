//! Interactive socket-native multimodal vision chat.
//!
//! The example keeps text/image history locally and sends OpenAI-style inline
//! `image_url` content parts to a vision-capable Supernet worker. It supports scene
//! description, OCR, object/spatial analysis, structured visual review, and
//! two-image comparison. This is not hosted REST chat or a durable chat run; state
//! ends with the process. Commands include `/image`, `/describe`, `/ocr`,
//! `/objects`, `/analyze`, `/compare`, `/clear-image`, `/clear`, `/history`,
//! `/system`, and `/stats`.
//!
//! Source files may be JPEG, PNG, WebP, or GIF and must be at most 20 MiB. JPEG/PNG
//! payloads of at most 10 MiB are preserved. Larger supported sources, WebP, and
//! the first GIF frame are decoded and encoded as PNG because the socket vision
//! contract accepts inline JPEG/PNG data URIs. The resulting PNG must also fit the
//! SDK's 10 MiB payload limit; resize the source if preprocessing exceeds it.
//!
//! Live use requires API-key or wallet-enabled username/password credentials and a
//! vision-capable worker. `--help` and dry run are credential-free; dry run may
//! still read and validate a local `--image`, but sends nothing. Pass `--execute`
//! to start the potentially paid session. Failed turns are removed from history.
//! Output streams visible assistant text and records token, timing, image, and
//! history statistics.
//!
//! # Examples
//!
//! ```text
//! cargo run --example workflow_text_chat_vision -- --help
//! cargo run --example workflow_text_chat_vision -- --dry-run --image examples/test-assets/placeholder.jpg
//! cargo run --example workflow_text_chat_vision -- --execute --image photo.webp
//! cargo run --example workflow_text_chat_vision -- --execute --system "Focus on typography and layout"
//! ```

#[path = "workflow_text_chat_vision/app.rs"]
mod app;
mod common;
#[path = "workflow_text_chat/shared/mod.rs"]
mod shared;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::run().await
}
