use crate::common::cli::{BillingMode, TokenType};
use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Mode {
    T2v,
    I2v,
    Flf2v,
    R2v,
}
impl Mode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::T2v => "t2v",
            Self::I2v => "i2v",
            Self::Flf2v => "flf2v",
            Self::R2v => "r2v",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum AudioPolicy {
    Reuse,
    Reference,
    Replace,
}
impl AudioPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reuse => "reuse",
            Self::Reference => "reference",
            Self::Replace => "replace",
        }
    }
}
#[derive(Debug, Parser)]
#[command(about = "MiniMax H3 T2V, I2V, FLF2V, and multi-reference R2V")]
pub struct Args {
    #[arg()]
    pub prompt: Option<String>,
    #[arg(long, conflicts_with = "prompt")]
    pub prompt_file: Option<PathBuf>,
    #[arg(long, value_enum)]
    pub mode: Option<Mode>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long = "image", alias = "first-image")]
    pub image: Option<PathBuf>,
    #[arg(long = "end-image", alias = "last-image")]
    pub end_image: Option<PathBuf>,
    #[arg(long = "ref-image")]
    pub ref_images: Vec<PathBuf>,
    #[arg(long = "ref-video")]
    pub ref_videos: Vec<PathBuf>,
    #[arg(long = "ref-audio")]
    pub ref_audios: Vec<PathBuf>,
    #[arg(long = "lora")]
    pub loras: Vec<String>,
    #[arg(long = "lora-strength", allow_hyphen_values = true)]
    pub lora_strengths: Vec<f64>,
    #[arg(long)]
    pub worker: Option<String>,
    #[arg(long, value_enum)]
    pub source_audio_policy: Option<AudioPolicy>,
    #[arg(long)]
    pub width: Option<u32>,
    #[arg(long)]
    pub height: Option<u32>,
    #[arg(long)]
    pub portrait: bool,
    #[arg(long)]
    pub duration: Option<f64>,
    #[arg(long)]
    pub frames: Option<i64>,
    #[arg(long, default_value_t = 1)]
    pub batch: u32,
    #[arg(long)]
    pub seed: Option<i64>,
    #[arg(long)]
    pub print_prompt: bool,
    #[arg(long="no-audio",action=clap::ArgAction::SetFalse,default_value_t=true)]
    pub generate_audio: bool,
    #[arg(long)]
    pub disable_safe_content_filter: bool,
    #[arg(long, value_enum, default_value = "spark")]
    pub token_type: TokenType,
    #[arg(long, alias = "billing", value_enum, default_value = "auto")]
    pub billing_mode: BillingMode,
    #[arg(long, default_value = "output")]
    pub output: PathBuf,
    #[arg(long)]
    pub execute: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub yes: bool,
}
pub use super::models::{Spec, resolved_mode, spec};
pub fn dimensions(args: &Args, spec: &Spec) -> Result<(u32, u32)> {
    let (default_w, default_h) = if args.portrait {
        (spec.height, spec.width)
    } else {
        (spec.width, spec.height)
    };
    let width = args.width.unwrap_or(default_w);
    let height = args.height.unwrap_or(default_h);
    if width == 0 || height == 0 || width % 32 != 0 || height % 32 != 0 {
        bail!("H3 dimensions must be positive multiples of 32");
    }
    if u64::from(width) * u64::from(height) > spec.max_pixels {
        bail!(
            "{width}x{height} exceeds this H3 model's {} pixel cap",
            spec.max_pixels
        );
    }
    Ok((width, height))
}
pub fn frames_and_duration(args: &Args) -> Result<(i64, f64)> {
    let frames = if let Some(value) = args.frames {
        if !(124..=362).contains(&value) || (value - 124) % 17 != 0 {
            bail!("frames must use the H3 grid 124 + n*17 in 124-362");
        }
        value
    } else {
        let requested = (args.duration.unwrap_or(8.0) * 24.0).round() as i64;
        let step = ((requested - 124) as f64 / 17.0).round() as i64;
        (124 + step * 17).clamp(124, 362)
    };
    Ok((frames, frames as f64 / 24.0))
}
pub fn validate_shape(args: &Args) -> Result<()> {
    let mode = resolved_mode(args)?;
    if args.loras.len() != args.lora_strengths.len() {
        bail!("provide exactly one --lora-strength for each --lora");
    }
    if !(1..=512).contains(&args.batch) {
        bail!("batch must be 1 through 512");
    }
    match mode {
        Mode::T2v if args.image.is_some() || args.end_image.is_some() => {
            bail!("t2v does not accept frame images")
        }
        Mode::I2v if args.image.is_none() && args.end_image.is_none() => {
            bail!("i2v requires --image and/or --end-image")
        }
        Mode::Flf2v if args.image.is_none() || args.end_image.is_none() => {
            bail!("flf2v requires --image and --end-image")
        }
        Mode::R2v if args.ref_images.is_empty() && args.ref_videos.is_empty() => {
            bail!("r2v requires at least one visual reference")
        }
        _ => {}
    }
    if mode != Mode::R2v
        && (!args.ref_images.is_empty()
            || !args.ref_videos.is_empty()
            || !args.ref_audios.is_empty())
    {
        bail!("--ref-* inputs are supported only in r2v mode");
    }
    if mode == Mode::R2v && (args.image.is_some() || args.end_image.is_some()) {
        bail!("r2v uses --ref-image, not frame anchors");
    }
    if args.ref_images.len() > 9
        || args.ref_videos.len() > 3
        || args.ref_audios.len() > 3
        || args.ref_images.len() + args.ref_videos.len() + args.ref_audios.len() > 12
    {
        bail!("H3 R2V limits are 9 images, 3 videos, 3 audios, and 12 files total");
    }
    let model = spec(args)?;
    dimensions(args, &model)?;
    frames_and_duration(args)?;
    Ok(())
}
