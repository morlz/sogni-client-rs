use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use sogni_client::is_ltx_model;

use crate::common::cli::{BillingMode, TokenType};

const DEFAULT_PROMPT: &str = "A ballerina in a pink tutu pirouettes underwater in sparkling dappled sunlight while a soothing violin melody plays.";

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Control {
    Canny,
    Pose,
    Depth,
    Detailer,
    Outpaint,
    Inpaint,
}

impl Control {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Canny => "canny",
            Self::Pose => "pose",
            Self::Depth => "depth",
            Self::Detailer => "detailer",
            Self::Outpaint => "outpaint",
            Self::Inpaint => "inpaint",
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Video-to-video control and WAN Animate workflows")]
pub struct Args {
    #[arg(default_value = DEFAULT_PROMPT)]
    pub prompt: String,
    #[arg(long, required = true)]
    pub video: Option<PathBuf>,
    #[arg(long)]
    pub image: Option<PathBuf>,
    #[arg(long, default_value = "ltx25-v2v-distilled")]
    pub model: String,
    #[arg(long="control-type",value_enum,default_value_t=Control::Canny)]
    pub control: Control,
    #[arg(long)]
    pub mask: Option<PathBuf>,
    #[arg(long)]
    pub outpaint_position: Option<String>,
    #[arg(long)]
    pub sam2_coords: Option<String>,
    #[arg(long)]
    pub video_start: Option<f64>,
    #[arg(long)]
    pub width: Option<u32>,
    #[arg(long)]
    pub height: Option<u32>,
    #[arg(long)]
    pub duration: Option<f64>,
    #[arg(long)]
    pub fps: Option<f64>,
    #[arg(long)]
    pub frames: Option<i64>,
    #[arg(long, default_value_t = 1)]
    pub batch: u32,
    #[arg(long)]
    pub seed: Option<u32>,
    #[arg(long)]
    pub steps: Option<u32>,
    #[arg(long)]
    pub guidance: Option<f64>,
    #[arg(long)]
    pub shift: Option<f64>,
    #[arg(long, default_value_t = 0.85)]
    pub strength: f64,
    #[arg(long)]
    pub detailer_strength: Option<f64>,
    #[arg(long = "comfy-sampler")]
    pub sampler: Option<String>,
    #[arg(long = "comfy-scheduler")]
    pub scheduler: Option<String>,
    #[arg(long)]
    pub negative: Option<String>,
    #[arg(long)]
    pub style: Option<String>,
    #[arg(long)]
    pub identity_audio: Option<PathBuf>,
    #[arg(long, requires = "identity_audio")]
    pub audio_identity_strength: Option<f64>,
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

pub struct Spec {
    pub id: String,
    pub control: bool,
    pub needs_image: bool,
    pub replace: bool,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub steps: u32,
    pub guidance: f64,
    pub shift: Option<f64>,
    pub sampler: &'static str,
    pub scheduler: &'static str,
    pub min: i64,
    pub max: i64,
    pub grid: u32,
    pub allows_extended: bool,
}

pub fn spec(value: &str) -> Result<Spec> {
    let id = match value {
        "ltx25-v2v-distilled" => "ltx25-22b-int8_v2v_distilled",
        "ltx25-v2v-dev" => "ltx25-22b-int8_v2v_dev",
        "ltx23-v2v-distilled" => "ltx23-22b-fp8_v2v_distilled",
        "ltx23-v2v-dev" => "ltx23-22b-fp8_v2v_dev",
        "move-lightx2v" => "wan_v2.2-14b-fp8_animate-move_lightx2v",
        "replace-lightx2v" => "wan_v2.2-14b-fp8_animate-replace_lightx2v",
        other => other,
    };
    let ltx = is_ltx_model(id);
    let animate = id.contains("_animate-");
    if !ltx && !animate {
        bail!("unsupported V2V model: {id}");
    }
    let fast = id.contains("distilled") || animate;
    Ok(Spec {
        id: id.into(),
        control: ltx,
        needs_image: animate,
        replace: id.contains("animate-replace"),
        width: if ltx { 1920 } else { 832 },
        height: if ltx { 1088 } else { 480 },
        fps: if ltx && id.starts_with("ltx23-") {
            25.0
        } else if ltx {
            24.0
        } else {
            16.0
        },
        steps: if ltx { if fast { 8 } else { 30 } } else { 6 },
        guidance: if fast { 1.0 } else { 3.0 },
        shift: animate.then_some(8.0),
        sampler: if fast { "euler_ancestral" } else { "euler" },
        scheduler: if id.starts_with("ltx25-") {
            "manual_sigmas"
        } else {
            "simple"
        },
        min: if ltx { 25 } else { 17 },
        max: if ltx { 505 } else { 321 },
        grid: if ltx { 64 } else { 16 },
        allows_extended: !id.starts_with("ltx25-") || id.contains("distilled"),
    })
}
