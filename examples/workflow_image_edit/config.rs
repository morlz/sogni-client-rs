use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

use crate::common::{
    cli::{BillingMode, TokenType, prompt, resolve_billing_mode, resolve_token_type, select},
    files::{choose_file, image_dimensions},
    models::{ImageModelSpec, edit_model, validate_image_options},
};

const DEFAULT_PROMPT: &str = "Generate an image in this style";
const EDIT_MODELS: &[(&str, &str)] = &[
    (
        "qwen-lightning",
        "fast general-purpose edit, up to 3 references",
    ),
    (
        "qwen",
        "higher-quality general-purpose edit, up to 3 references",
    ),
    (
        "krea-identity-edit",
        "preserve a person or character identity, 1-2 references",
    ),
    (
        "dark-beast-krea2-identity-edit",
        "community identity-preserving edit, 1-2 references",
    ),
];

#[derive(Debug, Parser)]
#[command(about = "Generate or edit an image from one or more references")]
pub struct Args {
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,
    #[arg(long = "context", alias = "image")]
    context: Vec<PathBuf>,
    #[arg(long)]
    context2: Option<PathBuf>,
    #[arg(long)]
    context3: Option<PathBuf>,
    #[arg(long)]
    context4: Option<PathBuf>,
    #[arg(long)]
    context5: Option<PathBuf>,
    #[arg(long)]
    context6: Option<PathBuf>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long, default_value_t = 1)]
    batch: u32,
    #[arg(long)]
    seed: Option<i64>,
    #[arg(long)]
    guidance: Option<f64>,
    #[arg(long)]
    steps: Option<u32>,
    #[arg(long)]
    sampler: Option<String>,
    #[arg(long)]
    scheduler: Option<String>,
    #[arg(long)]
    negative: Option<String>,
    #[arg(long)]
    style: Option<String>,
    #[arg(long, default_value = "examples/output")]
    output: PathBuf,
    #[arg(long, default_value = "jpg")]
    output_format: String,
    #[arg(long)]
    disable_safe_content_filter: bool,
    #[arg(long)]
    no_interactive: bool,
    #[arg(long, alias = "billing", value_enum)]
    billing_mode: Option<BillingMode>,
    #[arg(long, value_enum)]
    token_type: Option<TokenType>,
    #[arg(long)]
    execute: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

pub struct Config {
    pub prompt: String,
    pub contexts: Vec<PathBuf>,
    pub model: ImageModelSpec,
    pub width: u32,
    pub height: u32,
    pub batch: u32,
    pub seed: i64,
    pub guidance: f64,
    pub steps: u32,
    pub sampler: String,
    pub scheduler: String,
    pub negative: Option<String>,
    pub style: Option<String>,
    pub output: PathBuf,
    pub output_format: String,
    pub disable_filter: bool,
    pub billing_mode: BillingMode,
    pub token_type: TokenType,
    pub execute: bool,
    pub dry_run: bool,
    pub yes: bool,
}

impl Args {
    pub fn resolve(mut self) -> Result<Config> {
        let interactive = !self.no_interactive;
        for path in [
            self.context2,
            self.context3,
            self.context4,
            self.context5,
            self.context6,
        ]
        .into_iter()
        .flatten()
        {
            self.context.push(path);
        }
        let key = match self.model {
            Some(key) => key,
            None if interactive => {
                select("Select an image-edit model", EDIT_MODELS, "qwen-lightning")?
            }
            None => "qwen-lightning".into(),
        };
        let model = edit_model(&key)?;
        if self.context.is_empty() {
            self.context
                .push(choose_file(None, "reference image", interactive)?);
        }
        while interactive && self.context.len() < model.max_context_images {
            let next = prompt(
                &format!(
                    "Optional reference image {} (leave blank to finish)",
                    self.context.len() + 1
                ),
                Some(""),
            )?;
            if next.trim().is_empty() {
                break;
            }
            self.context.push(choose_file(
                Some(PathBuf::from(next)),
                "reference image",
                false,
            )?);
        }
        if self.context.len() > model.max_context_images {
            bail!(
                "{} accepts at most {} reference images",
                model.name,
                model.max_context_images
            );
        }
        for path in &self.context {
            choose_file(Some(path.clone()), "reference image", false)?;
        }
        let source_size = image_dimensions(&self.context[0]).unwrap_or((1024, 1024));
        let width = self.width.unwrap_or(source_size.0).min(model.max_width);
        let height = self.height.unwrap_or(source_size.1).min(model.max_height);
        let steps = self.steps.unwrap_or(model.default_steps);
        let guidance = self.guidance.unwrap_or(model.default_guidance);
        validate_image_options(model, width, height, steps, guidance)?;
        if !(1..=512).contains(&self.batch) {
            bail!("--batch must be between 1 and 512");
        }
        let prompt = match self.prompt {
            Some(value) => value,
            None if interactive => prompt("Edit prompt", Some(DEFAULT_PROMPT))?,
            None => DEFAULT_PROMPT.into(),
        };
        if prompt.trim().is_empty() {
            bail!("prompt cannot be empty");
        }
        let seed = self
            .seed
            .filter(|seed| *seed != -1)
            .unwrap_or_else(|| i64::from(rand::random::<u32>() & i32::MAX as u32));
        Ok(Config {
            prompt,
            contexts: self.context,
            model,
            width,
            height,
            batch: self.batch,
            seed,
            guidance,
            steps,
            sampler: self.sampler.unwrap_or_else(|| model.sampler.into()),
            scheduler: self.scheduler.unwrap_or_else(|| model.scheduler.into()),
            negative: self.negative,
            style: self.style,
            output: self.output,
            output_format: self
                .output_format
                .trim_start_matches('.')
                .to_ascii_lowercase(),
            disable_filter: self.disable_safe_content_filter,
            billing_mode: resolve_billing_mode(self.billing_mode)?,
            token_type: resolve_token_type(self.token_type, interactive && self.execute)?,
            execute: self.execute,
            dry_run: self.dry_run,
            yes: self.yes,
        })
    }
}
