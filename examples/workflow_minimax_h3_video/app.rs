mod config;
mod media;
mod models;
mod prompt;
use crate::common::{
    auth::{close, connect, unique_app_id},
    cli::{confirm_estimate, execution_requested, explain_dry_run},
    files::download_results,
    progress::wait_with_progress,
    workflow::{print_request, require_model},
};
use anyhow::Result;
use clap::Parser;
use config::{Args, Mode, dimensions, frames_and_duration, resolved_mode, spec, validate_shape};
use media::{Probed, no_probe, probe, remux_exact, reuse_source, validate_live_mode};
use serde_json::{Value, json};
use sogni_client::{AssetRole, MediaSource, Network, ProjectRequest};
fn build(args: &Args, probed: &Probed) -> Result<(ProjectRequest, i64)> {
    let mode = resolved_mode(args)?;
    let spec = spec(args)?;
    let (width, height) = dimensions(args, &spec)?;
    let (frames, duration) = frames_and_duration(args)?;
    let prompt = prompt::build(args, frames as f64 / 24.0, &probed.soundtracked)?;
    let prompt = if let Some(worker) = &args.worker {
        format!("{prompt} --workers={worker}")
    } else {
        prompt
    };
    let mut request = ProjectRequest::video(spec.id, prompt)
        .network(Network::Fast)
        .dimensions(width, height)
        .duration(duration)
        .fps(24.0)
        .number_of_media(args.batch)
        .steps(spec.steps)
        .guidance(1.0)
        .param("scheduler", "simple")
        .param("generateAudio", args.generate_audio)
        .param("safeContentFilter", !args.disable_safe_content_filter)
        .param("tokenType", args.token_type.as_str())
        .param("billingMode", args.billing_mode.as_str());
    if let Some(sampler) = spec.sampler {
        request = request.param("sampler", sampler);
    }
    if let Some(seed) = args.seed {
        request = request.param("seed", seed);
    }
    if !args.loras.is_empty() {
        request = request
            .param("loras", json!(args.loras))
            .param("loraStrengths", json!(args.lora_strengths));
    }
    match mode {
        Mode::I2v | Mode::Flf2v => {
            if let Some(path) = &args.image {
                request = request.asset(AssetRole::ReferenceImage, MediaSource::Path(path.clone()));
            }
            if let Some(path) = &args.end_image {
                request = request.asset(
                    AssetRole::ReferenceImageEnd,
                    MediaSource::Path(path.clone()),
                );
            }
        }
        Mode::R2v => {
            for (index, path) in args.ref_images.iter().enumerate() {
                request = request.asset(
                    if index == 0 {
                        AssetRole::ReferenceImage
                    } else {
                        AssetRole::ContextImage(index as u8)
                    },
                    MediaSource::Path(path.clone()),
                );
            }
            for (index, path) in args.ref_videos.iter().enumerate() {
                request = request.asset(
                    AssetRole::ReferenceVideoSlot((index + 1) as u8),
                    MediaSource::Path(path.clone()),
                );
            }
            for (index, path) in args.ref_audios.iter().enumerate() {
                request = request.asset(
                    AssetRole::ReferenceAudioSlot((index + 1) as u8),
                    MediaSource::Path(path.clone()),
                );
            }
            if !probed.video.is_empty() {
                request = request.param(
                    "referenceVideoDurations",
                    json!(
                        probed
                            .video
                            .iter()
                            .filter_map(|v| v.duration_seconds)
                            .collect::<Vec<_>>()
                    ),
                );
            }
        }
        Mode::T2v => {}
    }
    Ok((request, frames))
}
fn estimate(p: &Value, frames: i64, args: &Args, probed: &Probed) -> Value {
    json!({"tokenType":p["tokenType"],"model":p["modelId"],"width":p["width"],"height":p["height"],"frames":frames,"fps":24,"steps":p["steps"],"numberOfMedia":p["numberOfMedia"],"referenceImageCount":args.ref_images.len(),"referenceVideoCount":args.ref_videos.len(),"referenceVideoDurationSeconds":probed.video.iter().filter_map(|v|v.duration_seconds).sum::<f64>()})
}
pub async fn run() -> Result<()> {
    let args = Args::parse();
    validate_shape(&args)?;
    let mode = resolved_mode(&args)?;
    let execute = execution_requested(args.execute, args.dry_run)?;
    let probed = if execute {
        validate_live_mode(&args)?;
        probe(&args)?
    } else {
        no_probe(&args)
    };
    let (request, frames) = build(&args, &probed)?;
    if args.print_prompt {
        println!(
            "{}",
            request.params()["positivePrompt"]
                .as_str()
                .unwrap_or_default()
        );
        return Ok(());
    }
    if !execute {
        print_request(&request)?;
        explain_dry_run();
        return Ok(());
    }
    let client = connect(
        unique_app_id(&format!("sogni-rust-h3-{}", mode.as_str())),
        Network::Fast,
    )
    .await?;
    let result = async {
        let id = request.params()["modelId"]
            .as_str()
            .expect("model")
            .to_owned();
        require_model(&client.projects, &id).await?;
        let quote = client
            .projects
            .estimate_video_cost(&estimate(&request.params(), frames, &args, &probed))
            .await?;
        confirm_estimate(&quote, args.yes)?;
        let project = client.projects.create(request).await?;
        println!("Project: {}", project.id());
        let urls = wait_with_progress(&project).await?;
        let paths = download_results(&urls, &args.output, "minimax-h3", "mp4").await?;
        if let Some(source) = reuse_source(&args, &probed) {
            for path in paths {
                let backup = remux_exact(&path, &source)?;
                println!(
                    "Reused exact soundtrack; generated-audio backup: {}",
                    backup.display()
                );
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    result.and(close(&client).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_uses_duration_not_frames_and_numbered_reference_slots() {
        let args = Args::try_parse_from([
            "x",
            "--mode",
            "r2v",
            "--ref-image",
            "a.png",
            "--ref-video",
            "v.mp4",
            "--source-audio-policy",
            "reference",
        ])
        .unwrap();
        let probed = no_probe(&args);
        let (p, frames) = build(&args, &probed).unwrap();
        let value = p.params();
        assert_eq!(frames, 192);
        assert!(value.get("frames").is_none());
        assert_eq!(value["referenceImage"], true);
        assert_eq!(value["referenceVideo"], true);
        assert_eq!(value["fps"], 24.0);
    }
    #[test]
    fn fasth3_cannot_cross_modes() {
        let args = Args::try_parse_from([
            "x",
            "--mode",
            "r2v",
            "--model",
            "minimax-h3-fasth3-t2v-turbo",
            "--ref-image",
            "a.png",
        ])
        .unwrap();
        assert!(spec(&args).is_err());
    }
}
