use crate::common::cli::{BillingMode, TokenType};
use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Mode {
    T2v,
    I2v,
    Ia2v,
    V2v,
}
impl Mode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::T2v => "t2v",
            Self::I2v => "i2v",
            Self::Ia2v => "ia2v",
            Self::V2v => "v2v",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Target {
    Chat,
    Workflow,
}

#[derive(Debug, Parser)]
#[command(about = "Exercise Seedance through hosted chat or durable Creative Agent workflows")]
pub struct Args {
    #[arg(
        default_value = "A glass whale glides through a rain-slick neon city at night as the camera drifts alongside it, with premium cinematic lighting and graceful motion."
    )]
    pub prompt: String,
    #[arg(long, value_enum)]
    pub mode: Option<Mode>,
    #[arg(long, value_enum)]
    pub target: Option<Target>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, conflicts_with = "mini")]
    pub full: bool,
    #[arg(long, alias = "fast", conflicts_with = "full")]
    pub mini: bool,
    #[arg(long, default_value = "qwen3.6-35b-a3b-gguf-iq4xs")]
    pub llm_model: String,
    #[arg(long, default_value_t = 4.0)]
    pub duration: f64,
    #[arg(long, default_value_t = 24.0)]
    pub fps: f64,
    #[arg(long)]
    pub width: Option<u32>,
    #[arg(long)]
    pub height: Option<u32>,
    #[arg(
        long = "number",
        alias = "batch",
        alias = "variations",
        default_value_t = 1
    )]
    pub number: u32,
    #[arg(long)]
    pub seed: Option<u32>,
    #[arg(long = "negative-prompt", alias = "negative")]
    pub negative_prompt: Option<String>,
    #[arg(
        long = "image",
        alias = "context",
        alias = "context-image",
        alias = "reference-image"
    )]
    pub images: Vec<String>,
    #[arg(long)]
    pub end_image: Option<String>,
    #[arg(long = "audio", alias = "reference-audio")]
    pub audios: Vec<String>,
    #[arg(long)]
    pub audio_start: Option<f64>,
    #[arg(long = "audio-identity", alias = "voice")]
    pub audio_identity: Option<String>,
    #[arg(long)]
    pub audio_identity_strength: Option<f64>,
    #[arg(long = "video", alias = "reference-video")]
    pub videos: Vec<String>,
    #[arg(long)]
    pub video_start: Option<f64>,
    #[arg(long)]
    pub control_mode: Option<String>,
    #[arg(long)]
    pub first_frame_strength: Option<f64>,
    #[arg(long)]
    pub last_frame_strength: Option<f64>,
    #[arg(long, conflicts_with = "no_audio")]
    pub generate_audio: bool,
    #[arg(long, conflicts_with = "generate_audio")]
    pub no_audio: bool,
    #[arg(long)]
    pub no_expand_prompt: bool,
    #[arg(long)]
    pub no_estimate: bool,
    #[arg(long)]
    pub inspect_workflow: bool,
    #[arg(long)]
    pub watch: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long, value_enum, default_value = "spark")]
    pub token_type: TokenType,
    #[arg(long, alias = "billing", value_enum, default_value = "auto")]
    pub billing_mode: BillingMode,
    #[arg(long, default_value = "output")]
    pub output: PathBuf,
    #[arg(long)]
    pub execute: bool,
    #[arg(long, alias = "no-execute")]
    pub dry_run: bool,
    #[arg(long)]
    pub yes: bool,
}

impl Args {
    pub fn mode(&self) -> Mode {
        self.mode.unwrap_or_else(|| {
            if !self.videos.is_empty() {
                Mode::V2v
            } else if !self.audios.is_empty() {
                Mode::Ia2v
            } else if !self.images.is_empty() || self.end_image.is_some() {
                Mode::I2v
            } else {
                Mode::T2v
            }
        })
    }
    pub fn target(&self) -> Target {
        self.target.unwrap_or_else(|| {
            if self.mode() == Mode::T2v && !self.has_media() {
                Target::Chat
            } else {
                Target::Workflow
            }
        })
    }
    pub fn has_media(&self) -> bool {
        !self.images.is_empty()
            || !self.videos.is_empty()
            || !self.audios.is_empty()
            || self.end_image.is_some()
            || self.audio_identity.is_some()
    }
    pub fn model_id(&self) -> Result<&str> {
        let value = self.model.as_deref().unwrap_or(if self.full {
            "seedance2"
        } else {
            "seedance2-mini"
        });
        match value {
            "seedance2" | "seedance-2-0" => Ok("seedance-2-0"),
            "seedance2-mini" | "seedance-2-0-mini" => Ok("seedance-2-0-mini"),
            "seedance2-5" | "seedance-2-5" => Ok("seedance-2-5"),
            _ => bail!("unsupported Seedance model selector: {value}"),
        }
    }
    pub fn selector(&self) -> Result<&str> {
        match self.model_id()? {
            "seedance-2-0" => Ok("seedance2"),
            "seedance-2-0-mini" => Ok("seedance2-mini"),
            "seedance-2-5" => Ok("seedance2-5"),
            _ => unreachable!(),
        }
    }
    pub fn dimensions(&self) -> Result<(u32, u32)> {
        let low = self.model_id()?.contains("mini") || self.model_id()? == "seedance-2-5";
        Ok((
            self.width.unwrap_or(if low { 1280 } else { 1920 }),
            self.height.unwrap_or(if low { 720 } else { 1080 }),
        ))
    }
    pub fn generate_audio(&self) -> Option<bool> {
        if self.no_audio {
            Some(false)
        } else if self.generate_audio {
            Some(true)
        } else {
            None
        }
    }
}

pub fn validate(args: &Args) -> Result<()> {
    let mode = args.mode();
    let target = args.target();
    if target == Target::Chat && mode != Mode::T2v {
        bail!("media-bearing Seedance modes use --workflow");
    }
    if target == Target::Chat && args.has_media() {
        bail!("chat target is pure text-to-video; media requires --workflow");
    }
    let model = args.model_id()?;
    let max_duration = if model == "seedance-2-5" { 30.0 } else { 15.0 };
    if !(4.0..=max_duration).contains(&args.duration) {
        bail!("Seedance duration must be between 4 and {max_duration} seconds");
    }
    if args.fps != 24.0 {
        bail!("Seedance endpoint generation is fixed at 24fps");
    }
    if !(1..=16).contains(&args.number) {
        bail!("--number/--batch must be 1 through 16");
    }
    for (value, label, min, max) in [
        (args.first_frame_strength, "first-frame strength", 0.0, 1.0),
        (args.last_frame_strength, "last-frame strength", 0.0, 1.0),
        (
            args.audio_identity_strength,
            "audio identity strength",
            0.0,
            10.0,
        ),
    ] {
        if value.is_some_and(|v| v < min || v > max) {
            bail!("{label} must be {min} through {max}");
        }
    }
    if args.audio_start.is_some_and(|v| v < 0.0) || args.video_start.is_some_and(|v| v < 0.0) {
        bail!("media start offsets must be non-negative");
    }
    if args
        .control_mode
        .as_deref()
        .is_some_and(|v| v != "seedance-v2v")
    {
        bail!("this example supports only --control-mode seedance-v2v");
    }
    let (width, height) = args.dimensions()?;
    if model == "seedance-2-5" && width.max(height) > 1280 {
        bail!("Seedance 2.5 output is capped at the 720p tier (maximum dimension 1280)");
    }
    let counts = [
        args.images.len() + usize::from(args.end_image.is_some()),
        args.videos.len(),
        args.audios.len() + usize::from(args.audio_identity.is_some()),
    ];
    let limits = if model == "seedance-2-5" {
        [30, 10, 10, 50]
    } else {
        [9, 3, 3, 12]
    };
    for (count, limit, label) in [
        (counts[0], limits[0], "image"),
        (counts[1], limits[1], "video"),
        (counts[2], limits[2], "audio"),
    ] {
        if count > limit {
            bail!("Seedance supports at most {limit} {label} assets");
        }
    }
    if counts.iter().sum::<usize>() > limits[3] {
        bail!("Seedance supports at most {} total assets", limits[3]);
    }
    if model != "seedance-2-5" && counts[2] > 0 && counts[0] + counts[1] == 0 {
        bail!("Seedance audio references require at least one image or video reference");
    }
    match mode {
        Mode::I2v if args.images.is_empty() => bail!("i2v requires --image"),
        Mode::Ia2v if args.images.is_empty() || args.audios.is_empty() => {
            bail!("ia2v requires --image and --audio")
        }
        Mode::Ia2v if args.end_image.is_some() || args.audio_identity.is_some() => {
            bail!("ia2v does not accept --end-image or --audio-identity")
        }
        Mode::V2v if args.videos.is_empty() => bail!("v2v requires --video"),
        Mode::V2v if args.end_image.is_some() => bail!("v2v does not accept --end-image"),
        _ => {}
    }
    if target == Target::Workflow && args.seed.is_some() {
        bail!("durable hosted video tools do not expose a seed argument");
    }
    if target == Target::Workflow && args.audio_identity.is_some() {
        bail!("durable workflows accept registered persona voices, not direct audio identity URLs");
    }
    if target == Target::Workflow
        && mode == Mode::Ia2v
        && (counts[0] != 1 || counts[2] != 1 || counts[1] > 0)
    {
        bail!("durable sound_to_video accepts one image and one audio");
    }
    if target == Target::Workflow
        && mode == Mode::V2v
        && (counts[1] != 1 || counts[0] > 1 || counts[2] > 0)
    {
        bail!("durable video_to_video accepts one video and at most one image");
    }
    Ok(())
}
