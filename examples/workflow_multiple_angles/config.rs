use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::common::cli::{BillingMode, TokenType};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(super) enum Azimuth {
    #[default]
    Front,
    FrontRight,
    Right,
    BackRight,
    Back,
    BackLeft,
    Left,
    FrontLeft,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(super) enum Elevation {
    LowAngle,
    #[default]
    EyeLevel,
    Elevated,
    HighAngle,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(super) enum Distance {
    CloseUp,
    #[default]
    Medium,
    Wide,
}

impl Azimuth {
    const fn prompt(self) -> &'static str {
        match self {
            Self::Front => "front view",
            Self::FrontRight => "front-right quarter view",
            Self::Right => "right side view",
            Self::BackRight => "back-right quarter view",
            Self::Back => "back view",
            Self::BackLeft => "back-left quarter view",
            Self::Left => "left side view",
            Self::FrontLeft => "front-left quarter view",
        }
    }
}

impl Elevation {
    const fn prompt(self) -> &'static str {
        match self {
            Self::LowAngle => "low-angle shot",
            Self::EyeLevel => "eye-level shot",
            Self::Elevated => "elevated shot",
            Self::HighAngle => "high-angle shot",
        }
    }
}

impl Distance {
    const fn prompt(self) -> &'static str {
        match self {
            Self::CloseUp => "close-up",
            Self::Medium => "medium shot",
            Self::Wide => "wide shot",
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Generate one of 96 camera poses with the Multiple Angles LoRA")]
pub(super) struct Args {
    #[arg(value_name = "DESCRIPTION")]
    pub(super) description: Option<String>,
    #[arg(long, alias = "image")]
    pub(super) context: Option<PathBuf>,
    #[arg(long)]
    pub(super) model: Option<String>,
    #[arg(long, value_enum)]
    pub(super) azimuth: Option<Azimuth>,
    #[arg(long, value_enum)]
    pub(super) elevation: Option<Elevation>,
    #[arg(long, value_enum)]
    pub(super) distance: Option<Distance>,
    #[arg(long, default_value_t = 0.9)]
    pub(super) strength: f64,
    #[arg(long, alias = "anchor")]
    pub(super) anchor: Option<String>,
    #[arg(long)]
    pub(super) guidance: Option<f64>,
    #[arg(long, default_value_t = 1024)]
    pub(super) width: u32,
    #[arg(long, default_value_t = 1024)]
    pub(super) height: u32,
    #[arg(long, default_value_t = 1)]
    pub(super) batch: u32,
    #[arg(long)]
    pub(super) seed: Option<i64>,
    #[arg(long)]
    pub(super) steps: Option<u32>,
    #[arg(long, default_value = "examples/output")]
    pub(super) output: PathBuf,
    #[arg(long)]
    pub(super) disable_safe_content_filter: bool,
    #[arg(long)]
    pub(super) no_interactive: bool,
    #[arg(long, alias = "billing", value_enum)]
    pub(super) billing_mode: Option<BillingMode>,
    #[arg(long, value_enum)]
    pub(super) token_type: Option<TokenType>,
    #[arg(long)]
    pub(super) execute: bool,
    #[arg(long)]
    pub(super) dry_run: bool,
    #[arg(long)]
    pub(super) yes: bool,
}

pub(super) fn camera_prompt(
    azimuth: Azimuth,
    elevation: Elevation,
    distance: Distance,
    anchor: Option<&str>,
) -> String {
    // The activation keyword must lead the prompt for reliable LoRA triggering.
    let mut prompt = format!(
        "<sks> {} {} {}",
        azimuth.prompt(),
        elevation.prompt(),
        distance.prompt()
    );
    if let Some(anchor) = anchor.filter(|value| !value.trim().is_empty()) {
        prompt.push(' ');
        prompt.push_str(anchor.trim());
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_places_activation_keyword_first() {
        assert_eq!(
            camera_prompt(
                Azimuth::Back,
                Elevation::HighAngle,
                Distance::CloseUp,
                Some("red coat")
            ),
            "<sks> back view high-angle shot close-up red coat"
        );
    }
}
