use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

use crate::{
    common::{
        cli::{
            BillingMode, NetworkArg, TokenType, parse_csv, resolve_billing_mode, resolve_token_type,
        },
        models::{ImageModelSpec, TEXT_MODELS},
    },
    types::TierSteps,
};

const DEFAULT_PROMPT: &str = "A majestic snow-capped mountain range at golden hour with a crystal clear alpine lake in the foreground reflecting the peaks, photorealistic";

#[derive(Debug, Parser)]
#[command(about = "Benchmark text-to-image inference at tier step boundaries")]
pub struct Args {
    #[arg(long, value_enum, default_value_t)]
    network: NetworkArg,
    #[arg(long)]
    models: Option<String>,
    #[arg(long, default_value = DEFAULT_PROMPT)]
    prompt: String,
    #[arg(long, default_value_t = 3)]
    runs: usize,
    #[arg(long, default_value_t = 1)]
    warmup: usize,
    #[arg(long, default_value = "examples/output/benchmark")]
    output: PathBuf,
    #[arg(long)]
    no_download: bool,
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
    pub network: NetworkArg,
    pub models: Vec<ImageModelSpec>,
    pub prompt: String,
    pub runs: usize,
    pub warmup: usize,
    pub output: PathBuf,
    pub download: bool,
    pub billing_mode: BillingMode,
    pub token_type: TokenType,
    pub execute: bool,
    pub dry_run: bool,
    pub yes: bool,
}

impl Args {
    pub fn resolve(self) -> Result<Config> {
        if self.prompt.trim().is_empty() {
            bail!("--prompt cannot be empty");
        }
        let selected = self.models.as_deref().map(parse_csv);
        let mut models = Vec::new();
        for model in TEXT_MODELS {
            if selected
                .as_ref()
                .is_none_or(|keys| keys.iter().any(|key| key == model.key))
            {
                models.push(*model);
            }
        }
        if let Some(keys) = &selected {
            for key in keys {
                if !TEXT_MODELS.iter().any(|model| model.key == key) {
                    eprintln!("Warning: unknown model {key:?}; skipping it");
                }
            }
        }
        if models.is_empty() {
            bail!("no valid models selected");
        }
        let minimum_runs = self
            .warmup
            .checked_add(2)
            .ok_or_else(|| anyhow::anyhow!("--warmup is too large"))?;
        let runs = if self.runs < minimum_runs {
            eprintln!(
                "Increasing --runs from {} to {minimum_runs} so two measured runs remain",
                self.runs
            );
            minimum_runs
        } else {
            self.runs
        };
        Ok(Config {
            network: self.network,
            models,
            prompt: self.prompt,
            runs,
            warmup: self.warmup,
            output: self.output,
            download: !self.no_download,
            billing_mode: resolve_billing_mode(self.billing_mode)?,
            token_type: resolve_token_type(self.token_type, false)?,
            execute: self.execute,
            dry_run: self.dry_run,
            yes: self.yes,
        })
    }
}

pub fn tier(model_id: &str) -> Option<TierSteps> {
    let (min, max, default) = match model_id {
        "z_image_turbo_bf16" => (4, 10, 8),
        "z_image_bf16" => (20, 50, 25),
        "krea2_turbo_fp8_scaled" => (4, 12, 8),
        "chroma-v.46-flash_fp8" => (10, 20, 10),
        "chroma-v48-detail-svd_fp8" => (20, 40, 20),
        "chroma1-hd_fp8_scaled" => (20, 50, 26),
        "flux1-schnell-fp8" => (1, 5, 4),
        "qwen_image_2512_fp8_lightning" => (4, 8, 4),
        "qwen_image_2512_fp8" => (20, 50, 20),
        "dark_beast_z_image_turbo_v9_bf16" => (4, 12, 8),
        "dark_beast_krea2_fp8" => (8, 20, 16),
        "one_obsession_v22_fp16" => (20, 35, 28),
        _ => return None,
    };
    Some(TierSteps { min, max, default })
}
