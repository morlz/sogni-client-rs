use std::path::PathBuf;

use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

impl MediaKind {
    pub fn from_tool(name: &str) -> Option<Self> {
        match name {
            "generate_image" => Some(Self::Image),
            "generate_video" => Some(Self::Video),
            "generate_music" => Some(Self::Audio),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PipelineConfig {
    pub image_model: String,
    pub video_model: String,
    pub audio_model: String,
    pub quantity: u32,
    pub duration: Option<f64>,
    pub aspect_ratio: String,
    pub output_dir: PathBuf,
    pub assume_yes: bool,
    pub show_thinking: bool,
    pub token_type: String,
    pub billing_mode: String,
}

#[derive(Clone, Debug)]
pub struct ImageSpec {
    pub prompt: String,
    pub image_size: String,
}

#[derive(Clone, Debug)]
pub struct VideoSpec {
    pub prompt: String,
    pub camera_movement: String,
    pub shot_scale: String,
    pub style_anchor: String,
    pub stability_anchor: String,
}

#[derive(Clone, Debug)]
pub struct SongSpec {
    pub prompt: String,
    pub lyrics: String,
    pub bpm: u32,
    pub keyscale: String,
    pub timesignature: String,
    pub duration: f64,
    pub language: String,
}

#[derive(Clone, Debug)]
pub enum MediaSpec {
    Image(ImageSpec),
    Video(VideoSpec),
    Audio(SongSpec),
}

#[derive(Clone, Debug)]
pub struct GeneratedMedia {
    pub kind: MediaKind,
    pub model: String,
    pub prompt: String,
    pub files: Vec<PathBuf>,
}

pub fn string_arg(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn number_arg(args: &Map<String, Value>, key: &str) -> Option<f64> {
    args.get(key)
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
        .filter(|value| value.is_finite())
}

pub fn quantity_arg(args: &Map<String, Value>, fallback: u32) -> u32 {
    number_arg(args, "quantity")
        .map(|value| value.round().clamp(1.0, 512.0) as u32)
        .unwrap_or(fallback)
}
