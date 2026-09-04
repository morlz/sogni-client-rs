use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use sogni_client::{
    AssetRole, MediaSource, Network, ProjectRequest, calculate_video_frames, is_ltx_model,
    is_wan_model, new_id,
};

use crate::common::{
    auth::{close, connect, unique_app_id},
    cli::{confirm_estimate, execution_requested, explain_dry_run},
    files::{download_results, ffprobe, image_dimensions, require_file},
    progress::wait_with_progress,
    workflow::{print_request, require_model},
};

mod config;

use config::{Args, spec};

fn media_duration(path: &Path) -> f64 {
    match ffprobe(path) {
        Ok(meta) => meta.duration_seconds.unwrap_or(10.0),
        Err(error) => {
            eprintln!("Warning: {error}; using a 10 second duration fallback.");
            10.0
        }
    }
}

fn build(
    args: &Args,
    audio_path: PathBuf,
    detected_duration: Option<f64>,
) -> Result<ProjectRequest> {
    let config = spec(&args.model)?;
    if config.needs_image && args.image.is_none() {
        bail!("model {} requires --image", config.id);
    }
    if !config.needs_image && args.image.is_some() {
        bail!("A2V models do not accept --image");
    }
    if args.audio_start.is_some_and(|v| v < 0.0) || args.audio_duration.is_some_and(|v| v <= 0.0) {
        bail!("audio start must be non-negative and audio duration positive");
    }
    if args.batch == 0 || args.batch > 512 {
        bail!("batch must be 1 through 512");
    }
    let source_dims = args.image.as_deref().and_then(|p| image_dimensions(p).ok());
    let width = args
        .width
        .or(source_dims.map(|v| v.0))
        .unwrap_or(config.width)
        & !1;
    let height = args
        .height
        .or(source_dims.map(|v| v.1))
        .unwrap_or(config.height)
        & !1;
    if width < 480 || height < 480 {
        bail!("width and height must be at least 480");
    }
    // An explicit duration wins; otherwise a live run follows the source audio.
    let duration = args.duration.or(detected_duration).unwrap_or(10.0);
    let fps = args.fps.unwrap_or(config.fps);
    if is_wan_model(&config.id) && ![16.0, 32.0].contains(&fps) {
        bail!("WAN FPS must be 16 or 32");
    }
    if is_ltx_model(&config.id) && !(1.0..=60.0).contains(&fps) {
        bail!("LTX FPS must be 1 through 60");
    }
    // WAN remains tied to 16 generated fps even for interpolated 32 fps output;
    // LTX uses the requested rate and its native frame-step constraint.
    let frames = args.frames.unwrap_or(calculate_video_frames(
        &config.id,
        duration,
        fps,
        Some(config.min),
        Some(config.max),
    )?);
    let mut request = ProjectRequest::video(config.id, &args.prompt)
        .network(Network::Fast)
        .dimensions(width, height)
        .duration(duration)
        .fps(fps)
        .number_of_media(args.batch)
        .steps(args.steps.unwrap_or(config.steps))
        .guidance(args.guidance.unwrap_or(config.guidance))
        .param("frames", frames)
        .param(
            "sampler",
            args.sampler
                .clone()
                .unwrap_or_else(|| config.sampler.into()),
        )
        .param(
            "scheduler",
            args.scheduler
                .clone()
                .unwrap_or_else(|| config.scheduler.into()),
        )
        .param("safeContentFilter", !args.disable_safe_content_filter)
        .param("tokenType", args.token_type.as_str())
        .param("billingMode", args.billing_mode.as_str())
        .asset(AssetRole::ReferenceAudio, MediaSource::Path(audio_path));
    if let Some(image) = &args.image {
        request = request.asset(AssetRole::ReferenceImage, MediaSource::Path(image.clone()));
    }
    if let Some(v) = args.shift.or(config.shift) {
        request = request.param("shift", v);
    }
    if let Some(v) = args.seed {
        request = request.param("seed", v);
    }
    if let Some(v) = args.audio_start {
        request = request.param("audioStart", v);
    }
    if let Some(v) = args.audio_duration {
        request = request.param("audioDuration", v);
    }
    if let Some(v) = &args.negative {
        request = request.param("negativePrompt", v.clone());
    }
    if let Some(v) = &args.style {
        request = request.param("stylePrompt", v.clone());
    }
    Ok(request)
}

struct PreparedAudio {
    path: PathBuf,
    temporary: bool,
}
impl Drop for PreparedAudio {
    fn drop(&mut self) {
        if self.temporary {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn prepare_audio(path: &Path) -> Result<PreparedAudio> {
    if path
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("m4a"))
    {
        return Ok(PreparedAudio {
            path: path.to_owned(),
            temporary: false,
        });
    }
    // Current video workers consume the driving track as M4A. Convert before
    // upload and let PreparedAudio remove only the temporary derivative.
    let output = std::env::temp_dir().join(format!("sogni-s2v-{}.m4a", new_id()));
    let result = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-vn",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ])
        .arg(&output)
        .output();
    match result {
        Ok(value) if value.status.success() => Ok(PreparedAudio {
            path: output,
            temporary: true,
        }),
        Ok(value) => bail!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&value.stderr).trim()
        ),
        Err(error) => {
            bail!("could not run ffmpeg ({error}); install FFmpeg or provide an .m4a audio file")
        }
    }
}
fn estimate(p: &Value) -> Value {
    json!({"tokenType":p["tokenType"],"model":p["modelId"],"width":p["width"],"height":p["height"],"frames":p["frames"],"fps":p["fps"],"steps":p["steps"],"numberOfMedia":p["numberOfMedia"]})
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let input = args.audio.as_ref().expect("required audio");
    let execute = execution_requested(args.execute, args.dry_run)?;
    // Dry-run request construction never probes or reads the named local file.
    let duration = if execute {
        require_file(input, "audio")?;
        Some(media_duration(input))
    } else {
        args.duration
    };
    let preview = build(&args, input.clone(), duration)?;
    if !execute {
        print_request(&preview)?;
        explain_dry_run();
        return Ok(());
    }
    if let Some(p) = &args.image {
        require_file(p, "image")?;
    }
    let client = connect(unique_app_id("sogni-rust-s2v"), Network::Fast).await?;
    let result = async {
        let prepared = prepare_audio(input).context("prepare worker-compatible audio")?;
        let request = build(&args, prepared.path.clone(), duration)?;
        let id = request.params()["modelId"]
            .as_str()
            .expect("model")
            .to_owned();
        require_model(&client.projects, &id).await?;
        // Estimate the exact post-conversion request before project creation.
        let quote = client
            .projects
            .estimate_video_cost(&estimate(&request.params()))
            .await?;
        confirm_estimate(&quote, args.yes)?;
        let project = client.projects.create(request).await?;
        println!("Project: {}", project.id());
        let urls = wait_with_progress(&project).await?;
        download_results(&urls, &args.output, "sound-to-video", "mp4").await?;
        Result::<()>::Ok(())
    }
    .await;
    result.and(close(&client).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a2v_rejects_image_and_ia2v_requires_it() {
        let a = Args::try_parse_from([
            "x",
            "--audio",
            "a.m4a",
            "--model",
            "ltx25-22b-int8_a2v_distilled",
            "--image",
            "i.png",
        ])
        .unwrap();
        assert!(build(&a, "a.m4a".into(), Some(5.0)).is_err());
        let b = Args::try_parse_from([
            "x",
            "--audio",
            "a.m4a",
            "--model",
            "ltx25-22b-int8_ia2v_distilled",
        ])
        .unwrap();
        assert!(build(&b, "a.m4a".into(), Some(5.0)).is_err());
    }

    #[test]
    fn quality_alias_builds_wan_s2v_request() {
        let args = Args::try_parse_from([
            "x", "--audio", "a.m4a", "--image", "i.png", "--model", "quality",
        ])
        .expect("quality arguments");
        let request = build(&args, "a.m4a".into(), Some(5.0)).expect("quality S2V request");
        let params = request.params();

        assert_eq!(params["modelId"], "wan_v2.2-14b-fp8_s2v");
        assert_eq!(params["frames"], 81);
        assert_eq!(params["steps"], 20);
        assert_eq!(params["referenceImage"], true);
        assert_eq!(params["referenceAudio"], true);
    }
}
