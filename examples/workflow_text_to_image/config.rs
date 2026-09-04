use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

use crate::common::{
    cli::{BillingMode, TokenType, prompt, resolve_billing_mode, resolve_token_type, select},
    files::{choose_file, slug},
    models::{ImageModelSpec, TEXT_MODELS, text_model, validate_image_options},
};

const DEFAULT_PROMPT: &str = "A mixed-media scene combining realistic photography and hand-drawn illustration. A 2D chibi cute cat is suggested only by loose, broken ink strokes, standing quietly in real physical space. The silhouette is fragmented and incomplete, made from varied ink strokes, hatching, and open negative space against a realistic beach with shallow depth of field. The mood is quiet, fragile, and melancholic.";

#[derive(Debug, Parser)]
#[command(about = "Generate images from text with Sogni image models")]
pub struct Args {
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long, default_value_t = 1)]
    batch: u32,
    #[arg(long)]
    guidance: Option<f64>,
    #[arg(long)]
    steps: Option<u32>,
    #[arg(long)]
    seed: Option<i64>,
    #[arg(long)]
    sampler: Option<String>,
    #[arg(long)]
    scheduler: Option<String>,
    #[arg(long)]
    negative: Option<String>,
    #[arg(long)]
    style: Option<String>,
    #[arg(long)]
    starting_image: Option<PathBuf>,
    #[arg(long, default_value_t = 0.5)]
    strength: f64,
    #[arg(long)]
    style_lora: Option<String>,
    #[arg(long, default_value_t = 1.0)]
    lora_strength: f64,
    #[arg(long, default_value_t = 0)]
    previews: u32,
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

#[derive(Debug)]
pub struct Config {
    pub prompt: String,
    pub model: ImageModelSpec,
    pub width: u32,
    pub height: u32,
    pub batch: u32,
    pub guidance: f64,
    pub steps: u32,
    pub seed: i64,
    pub sampler: String,
    pub scheduler: String,
    pub negative: Option<String>,
    pub style: Option<String>,
    pub starting_image: Option<PathBuf>,
    pub strength: f64,
    pub style_lora: Option<String>,
    pub lora_strength: f64,
    pub previews: u32,
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
    pub fn resolve(self) -> Result<Config> {
        let interactive = !self.no_interactive;
        let key = match self.model {
            Some(key) => key,
            None if interactive => select(
                "Select an image model",
                &TEXT_MODELS
                    .iter()
                    .map(|model| (model.key, model.name))
                    .collect::<Vec<_>>(),
                "z-turbo",
            )?,
            None => "z-turbo".into(),
        };
        let model = text_model(&key)?;
        let prompt_value = match self.prompt {
            Some(prompt) => prompt,
            None if interactive => prompt("Positive prompt", Some(DEFAULT_PROMPT))?,
            None => DEFAULT_PROMPT.into(),
        };
        if prompt_value.trim().is_empty() {
            bail!("prompt cannot be empty");
        }
        let width = aligned(self.width.unwrap_or(model.width).min(model.max_width))?;
        let height = aligned(self.height.unwrap_or(model.height).min(model.max_height))?;
        let steps = self.steps.unwrap_or(model.default_steps);
        let guidance = self.guidance.unwrap_or(model.default_guidance);
        validate_image_options(model, width, height, steps, guidance)?;
        if !(1..=512).contains(&self.batch) {
            bail!("--batch must be between 1 and 512");
        }
        if self.previews > 20 {
            bail!("--previews must be between 0 and 20");
        }
        if !(0.0..=1.0).contains(&self.strength) {
            bail!("--strength must be between 0 and 1");
        }
        if !self.lora_strength.is_finite() || !(0.0..=2.0).contains(&self.lora_strength) {
            bail!("--lora-strength must be between 0 and 2");
        }
        let starting_image = match self.starting_image {
            Some(path) if model.supports_starting_image => {
                Some(choose_file(Some(path), "starting image", false)?)
            }
            Some(_) => {
                eprintln!(
                    "{} does not support starting images; ignoring it",
                    model.name
                );
                None
            }
            None => None,
        };
        let seed = self
            .seed
            .filter(|seed| *seed != -1)
            .unwrap_or_else(|| i64::from(rand::random::<u32>() & i32::MAX as u32));
        Ok(Config {
            prompt: prompt_value,
            model,
            width,
            height,
            batch: self.batch,
            guidance,
            steps,
            seed,
            sampler: self.sampler.unwrap_or_else(|| model.sampler.into()),
            scheduler: self.scheduler.unwrap_or_else(|| model.scheduler.into()),
            negative: self
                .negative
                .or_else(|| model.negative_prompt.map(ToOwned::to_owned)),
            style: self.style,
            starting_image,
            strength: self.strength,
            style_lora: self.style_lora,
            lora_strength: self.lora_strength,
            previews: self.previews,
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

    #[cfg(test)]
    pub fn resolve_for_test(self) -> Result<Config> {
        self.resolve()
    }
}

fn aligned(value: u32) -> Result<u32> {
    let value = value / 16 * 16;
    if value == 0 {
        bail!("image dimensions must be at least 16 pixels");
    }
    Ok(value)
}

impl Config {
    pub fn filename_prefix(&self) -> String {
        format!(
            "{}-{}x{}-{}-{}",
            self.model.key,
            self.width,
            self.height,
            self.seed,
            slug(&self.prompt, 30)
        )
    }
}
