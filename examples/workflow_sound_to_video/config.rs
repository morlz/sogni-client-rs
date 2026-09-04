use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use sogni_client::{get_video_workflow_type, is_ltx_model};

use crate::common::cli::{BillingMode, TokenType};

const DEFAULT_PROMPT: &str =
    "A person speaking naturally with synchronized lip movements to the audio";

#[derive(Debug, Parser)]
#[command(about = "Drive a WAN or LTX video with audio")]
pub(super) struct Args {
    #[arg(default_value=DEFAULT_PROMPT)]
    pub(super) prompt: String,
    #[arg(long, required = true)]
    pub(super) audio: Option<PathBuf>,
    #[arg(long)]
    pub(super) image: Option<PathBuf>,
    #[arg(long, default_value = "lightx2v")]
    pub(super) model: String,
    #[arg(long)]
    pub(super) width: Option<u32>,
    #[arg(long)]
    pub(super) height: Option<u32>,
    #[arg(long)]
    pub(super) duration: Option<f64>,
    #[arg(long)]
    pub(super) fps: Option<f64>,
    #[arg(long)]
    pub(super) frames: Option<i64>,
    #[arg(long, default_value_t = 1)]
    pub(super) batch: u32,
    #[arg(long)]
    pub(super) audio_start: Option<f64>,
    #[arg(long)]
    pub(super) audio_duration: Option<f64>,
    #[arg(long)]
    pub(super) seed: Option<u32>,
    #[arg(long)]
    pub(super) steps: Option<u32>,
    #[arg(long)]
    pub(super) guidance: Option<f64>,
    #[arg(long)]
    pub(super) shift: Option<f64>,
    #[arg(long = "comfy-sampler")]
    pub(super) sampler: Option<String>,
    #[arg(long = "comfy-scheduler")]
    pub(super) scheduler: Option<String>,
    #[arg(long)]
    pub(super) negative: Option<String>,
    #[arg(long)]
    pub(super) style: Option<String>,
    #[arg(long)]
    pub(super) disable_safe_content_filter: bool,
    #[arg(long, value_enum, default_value = "spark")]
    pub(super) token_type: TokenType,
    #[arg(long, alias = "billing", value_enum, default_value = "auto")]
    pub(super) billing_mode: BillingMode,
    #[arg(long, default_value = "output")]
    pub(super) output: PathBuf,
    #[arg(long)]
    pub(super) execute: bool,
    #[arg(long)]
    pub(super) dry_run: bool,
    #[arg(long)]
    pub(super) yes: bool,
}

pub(super) struct Spec {
    pub(super) id: String,
    pub(super) needs_image: bool,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) fps: f64,
    pub(super) steps: u32,
    pub(super) guidance: f64,
    pub(super) shift: Option<f64>,
    pub(super) sampler: &'static str,
    pub(super) scheduler: &'static str,
    pub(super) min: i64,
    pub(super) max: i64,
}

pub(super) fn spec(value: &str) -> Result<Spec> {
    let id = match value {
        "lightx2v" => "wan_v2.2-14b-fp8_s2v_lightx2v",
        "quality" => "wan_v2.2-14b-fp8_s2v",
        "ltx23-ia2v-distilled" => "ltx23-22b-fp8_ia2v_distilled",
        "ltx23-ia2v-dev" => "ltx23-22b-fp8_ia2v_dev",
        "ltx23-a2v-distilled" => "ltx23-22b-fp8_a2v_distilled",
        "ltx23-a2v-dev" => "ltx23-22b-fp8_a2v_dev",
        other => other,
    };
    let workflow = get_video_workflow_type(id)
        .ok_or_else(|| anyhow::anyhow!("unsupported audio-driven model: {id}"))?;
    if !matches!(workflow, "s2v" | "ia2v" | "a2v") {
        bail!("model {id} is not S2V, IA2V, or A2V");
    }
    let ltx = is_ltx_model(id);
    let fast = id.contains("distilled") || id.contains("lightx2v");
    Ok(Spec {
        id: id.into(),
        needs_image: workflow != "a2v",
        width: if ltx { 1920 } else { 832 },
        height: if ltx { 1088 } else { 480 },
        fps: if ltx { 24.0 } else { 16.0 },
        steps: if ltx {
            if fast { 8 } else { 30 }
        } else if fast {
            4
        } else {
            20
        },
        guidance: if fast {
            1.0
        } else if ltx {
            3.0
        } else {
            6.0
        },
        shift: (!ltx).then_some(8.0),
        sampler: if ltx { "euler_ancestral" } else { "uni_pc" },
        scheduler: if id.starts_with("ltx25-") {
            "manual_sigmas"
        } else if ltx {
            "normal"
        } else {
            "simple"
        },
        min: if ltx { 25 } else { 17 },
        max: if ltx { 505 } else { 321 },
    })
}
