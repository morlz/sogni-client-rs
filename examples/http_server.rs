mod common;
#[path = "http_server/server.rs"]
mod server;

use anyhow::Result;
use clap::Parser as _;

#[tokio::main]
async fn main() -> Result<()> {
    server::run(server::Args::parse()).await
}
