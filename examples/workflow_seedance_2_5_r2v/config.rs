use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum TaskType {
    Reference,
    Edit,
    Extend,
}

impl TaskType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Edit => "edit",
            Self::Extend => "extend",
        }
    }

    const fn default_prompt(self) -> &'static str {
        match self {
            Self::Reference => {
                "Use @Image1 for the subject and @Video1 for camera motion in a cohesive new clip."
            }
            Self::Edit => {
                "Edit @Video1. Change the environment while preserving the subject, timing, and sound."
            }
            Self::Extend => {
                "Extend @Video1 after its ending; preserve the cast, scene, pacing, and sound."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Layer {
    Direct,
    CreativeAgent,
}

#[derive(Debug, Parser)]
#[command(about = "Seedance 2.5 reference, edit, and extend workflows")]
pub struct Args {
    /// Prompt using @Image1, @Video1, and @Audio1 references.
    pub prompt: Option<String>,
    #[arg(long, value_enum, default_value_t = TaskType::Reference)]
    pub task_type: TaskType,
    #[arg(long, alias = "api", value_enum, default_value_t = Layer::Direct)]
    pub layer: Layer,
    #[arg(long, default_value = "seedance-2-5")]
    pub model: String,
    /// New duration; direct edit requires this explicitly and it must match @Video1.
    #[arg(long)]
    pub duration: Option<f64>,
    #[arg(long, default_value = "720p")]
    pub resolution: String,
    #[arg(long = "image", alias = "reference-image")]
    pub images: Vec<String>,
    #[arg(long = "video", alias = "reference-video")]
    pub videos: Vec<String>,
    #[arg(long = "audio", alias = "reference-audio")]
    pub audios: Vec<String>,
    #[arg(long = "number", alias = "batch", default_value_t = 1)]
    pub number_of_media: u32,
    #[arg(long = "no-audio", action = clap::ArgAction::SetFalse, default_value_t = true)]
    pub generate_audio: bool,
    #[arg(long)]
    pub watch: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long, default_value = "output")]
    pub output: PathBuf,
    /// Opt in to uploads and paid generation.
    #[arg(long)]
    pub execute: bool,
    #[arg(long, alias = "no-execute")]
    pub dry_run: bool,
    #[arg(long)]
    pub yes: bool,
}

impl Args {
    pub fn prompt(&self) -> &str {
        self.prompt
            .as_deref()
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
            .unwrap_or(self.task_type.default_prompt())
    }
}

#[derive(Clone, Copy)]
pub struct ModelConfig {
    pub selector: Option<&'static str>,
    pub max_duration: f64,
    pub resolutions: &'static [&'static str],
    pub limits: [usize; 4],
    pub supports_task_type: bool,
    pub audio_only: bool,
}

pub fn model_config(model: &str) -> Result<ModelConfig> {
    let config = match model {
        "seedance-2-5" => ModelConfig {
            selector: Some("seedance2-5"),
            max_duration: 30.0,
            resolutions: &["480p", "720p"],
            limits: [30, 10, 10, 50],
            supports_task_type: true,
            audio_only: true,
        },
        "seedance-2-0" => ModelConfig {
            selector: Some("seedance2"),
            max_duration: 15.0,
            resolutions: &["480p", "720p", "1080p", "4k"],
            limits: [9, 3, 3, 12],
            supports_task_type: false,
            audio_only: false,
        },
        "seedance-2-0-mini" => ModelConfig {
            selector: Some("seedance2-mini"),
            max_duration: 15.0,
            resolutions: &["480p", "720p"],
            limits: [9, 3, 3, 12],
            supports_task_type: false,
            audio_only: false,
        },
        "seedance-2-0-fast" => ModelConfig {
            selector: None,
            max_duration: 15.0,
            resolutions: &["480p", "720p"],
            limits: [9, 3, 3, 12],
            supports_task_type: false,
            audio_only: false,
        },
        _ => bail!(
            "--model must be seedance-2-5, seedance-2-0, seedance-2-0-mini, or seedance-2-0-fast"
        ),
    };
    Ok(config)
}

pub fn duration(args: &Args) -> f64 {
    args.duration.unwrap_or(5.0)
}

pub fn dimensions(resolution: &str) -> Result<(u32, u32)> {
    match resolution {
        "480p" => Ok((864, 496)),
        "720p" => Ok((1280, 720)),
        "1080p" => Ok((1920, 1080)),
        "4k" => Ok((3840, 2160)),
        _ => bail!("--resolution must be 480p, 720p, 1080p, or 4k"),
    }
}

pub fn validate(args: &Args) -> Result<()> {
    let config = model_config(&args.model)?;
    if !config.resolutions.contains(&args.resolution.as_str()) {
        bail!(
            "{} supports {} output, not {}",
            args.model,
            config.resolutions.join("/"),
            args.resolution
        );
    }
    let duration = duration(args);
    if !(4.0..=config.max_duration).contains(&duration) {
        bail!(
            "{} duration must be between 4 and {} seconds",
            args.model,
            config.max_duration
        );
    }
    if !(1..=16).contains(&args.number_of_media) {
        bail!("--number/--batch must be 1 through 16");
    }
    let counts = [args.images.len(), args.videos.len(), args.audios.len()];
    for (count, limit, label) in [
        (counts[0], config.limits[0], "images"),
        (counts[1], config.limits[1], "videos"),
        (counts[2], config.limits[2], "audios"),
    ] {
        if count > limit {
            bail!("{} supports at most {limit} {label}", args.model);
        }
    }
    if counts.iter().sum::<usize>() > config.limits[3] {
        bail!(
            "{} supports at most {} total media files",
            args.model,
            config.limits[3]
        );
    }
    if args.task_type == TaskType::Reference && counts.iter().sum::<usize>() == 0 {
        bail!("reference requires at least one loose image, video, or audio reference");
    }
    // Edit and extend express a relationship to @Video1; loose references alone
    // cannot silently change either operation into reference generation.
    if matches!(args.task_type, TaskType::Edit | TaskType::Extend) && args.videos.is_empty() {
        bail!(
            "{} requires at least one source video as @Video1",
            args.task_type.as_str()
        );
    }
    if args.layer == Layer::Direct && args.task_type == TaskType::Edit && args.duration.is_none() {
        bail!("Direct edit requires --duration set to @Video1's source duration");
    }
    // The 2.5 task discriminator is not part of the legacy 2.0 wire contract.
    if !config.supports_task_type && args.task_type != TaskType::Reference {
        bail!(
            "{} is exposed by this example only for Seedance 2.5",
            args.task_type.as_str()
        );
    }
    if !config.audio_only
        && !args.audios.is_empty()
        && args.images.is_empty()
        && args.videos.is_empty()
    {
        bail!(
            "{} audio references require at least one image or video reference",
            args.model
        );
    }
    if args.layer == Layer::CreativeAgent {
        if config.selector.is_none() {
            bail!(
                "{} has no Creative Agent selector; use --layer direct",
                args.model
            );
        }
        if args.task_type == TaskType::Edit
            && (args.videos.len() != 1 || args.images.len() > 1 || !args.audios.is_empty())
        {
            bail!("Creative Agent edit accepts one source video and at most one source image");
        }
        if args.task_type == TaskType::Extend
            && (args.videos.len() != 1 || !args.images.is_empty() || !args.audios.is_empty())
        {
            bail!(
                "Creative Agent extend accepts exactly one source video and no supplemental media"
            );
        }
    }
    dimensions(&args.resolution)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_prompt_is_parsed_then_resolved_for_the_selected_task() {
        for (task, expected) in [
            (
                "reference",
                "Use @Image1 for the subject and @Video1 for camera motion in a cohesive new clip.",
            ),
            (
                "edit",
                "Edit @Video1. Change the environment while preserving the subject, timing, and sound.",
            ),
            (
                "extend",
                "Extend @Video1 after its ending; preserve the cast, scene, pacing, and sound.",
            ),
        ] {
            let options = Args::try_parse_from(["example", "--task-type", task]).unwrap();
            assert!(options.prompt.is_none());
            assert_eq!(options.prompt(), expected);
        }
    }
}
