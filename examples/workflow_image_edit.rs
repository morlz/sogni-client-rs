//! Edit an image from ordered reference images using Qwen Image Edit or Krea 2
//! Identity Edit model families.
//!
//! Choose the model for what must be preserved: Krea Identity Edit is intended
//! for recognizable people/characters, while Qwen is the general editor for
//! transforms, text, multi-person changes, and combining references. References
//! are uploaded into numbered context slots in CLI order. Dimensions default to
//! the first source image (within model limits); steps, guidance, sampler, and
//! scheduler use model-family defaults unless explicitly overridden. `--image`
//! remains a legacy alias for the first `--context`. Output filenames include
//! the resolved request seed.
//!
//! `--help` is credential-free. Dry runs validate/read local references but send
//! no network request. Live generation requires credentials, `--execute`, and
//! cost confirmation unless `--yes` is supplied; results are downloaded under
//! `examples/output` by default.
//!
//! ```text
//! cargo run --example workflow_image_edit -- --help
//! cargo run --example workflow_image_edit -- "Turn this into watercolor" --context input.png --model qwen-lightning --no-interactive --dry-run
//! cargo run --example workflow_image_edit -- "Keep the subject, change the outfit" --context portrait.jpg --model krea-identity-edit --no-interactive --execute
//! cargo run --example workflow_image_edit -- "Combine these references" --context scene.jpg --context subject.jpg --model qwen --no-interactive --execute --yes
//! ```

#[path = "workflow_image_edit/app.rs"]
mod app;
mod common;
#[path = "workflow_image_edit/config.rs"]
mod config;

use anyhow::Result;
use clap::Parser as _;

#[tokio::main]
async fn main() -> Result<()> {
    app::run(config::Args::parse()).await
}
