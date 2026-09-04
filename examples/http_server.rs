//! Serve a small production-shaped Axum frontend for Sogni image generation.
//!
//! The server exposes `GET /`, `GET /healthz`, `POST /api/estimate`, and
//! `POST /api/generate`. Generation requires the browser to confirm a current
//! estimate; bounded request bodies, prompt validation, concurrency limiting,
//! timeouts, security headers, and graceful shutdown keep the example suitable
//! as an application foundation. It binds to loopback by default. Do not expose
//! it publicly without adding authentication, authorization, rate limits, and
//! your own billing policy; non-loopback binding requires `--allow-remote`.
//!
//! `--help` and the default dry run need no credentials or listener. Starting
//! the live server requires Sogni credentials and `--execute`; generated images
//! are returned as hosted result URLs to the page.
//!
//! ```text
//! cargo run --example http_server -- --help
//! cargo run --example http_server -- --model flux1-schnell-fp8 --dry-run
//! cargo run --example http_server -- --execute
//! cargo run --example http_server -- --listen 127.0.0.1:8080 --max-concurrent 4 --execute
//! ```

mod common;
#[path = "http_server/server.rs"]
mod server;

use anyhow::Result;
use clap::Parser as _;

#[tokio::main]
async fn main() -> Result<()> {
    server::run(server::Args::parse()).await
}
