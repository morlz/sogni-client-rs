use anyhow::{Result, bail};
use serde_json::json;
use sogni_client::{Network, ProjectRequest, SogniClient};

use crate::common;

use super::types::{
    GeneratedMedia, ImageSpec, MediaKind, MediaSpec, PipelineConfig, SongSpec, VideoSpec,
};

const IMAGE_STEPS: u32 = 8;
const VIDEO_STEPS: u32 = 20;
const AUDIO_STEPS: u32 = 8;

pub async fn generate(
    client: &SogniClient,
    spec: MediaSpec,
    config: &PipelineConfig,
    quantity: u32,
    duration_override: Option<f64>,
    aspect_override: Option<&str>,
) -> Result<GeneratedMedia> {
    match spec {
        MediaSpec::Image(spec) => generate_image(client, spec, config, quantity).await,
        MediaSpec::Video(spec) => {
            generate_video(
                client,
                spec,
                config,
                quantity,
                duration_override,
                aspect_override,
            )
            .await
        }
        MediaSpec::Audio(mut spec) => {
            if let Some(duration) = duration_override {
                spec.duration = duration.clamp(10.0, 600.0);
            }
            generate_audio(client, spec, config, quantity).await
        }
    }
}

async fn generate_image(
    client: &SogniClient,
    spec: ImageSpec,
    config: &PipelineConfig,
    quantity: u32,
) -> Result<GeneratedMedia> {
    let (width, height) = image_dimensions(&spec.image_size);
    confirm_quote(
        client
            .projects
            .estimate_cost(&json!({
                "tokenType": config.token_type,
                "network": "fast",
                "model": config.image_model,
                "imageCount": quantity,
                "stepCount": IMAGE_STEPS,
                "previewCount": 0,
                "cnEnabled": false,
                "startingImageStrength": 0,
                "width": width,
                "height": height,
                "guidance": 1,
                "sampler": "euler"
            }))
            .await,
        config.assume_yes,
    )?;
    println!(
        "Generating {quantity} image(s) with {} ({width}x{height})...",
        config.image_model
    );
    let request = ProjectRequest::image(&config.image_model, &spec.prompt)
        .network(Network::Fast)
        .number_of_media(quantity)
        .dimensions(width, height)
        .steps(IMAGE_STEPS)
        .guidance(1.0)
        .param("sampler", "euler")
        .param("scheduler", "simple")
        .param("seed", -1)
        .param("outputFormat", "jpg")
        .param("tokenType", config.token_type.clone())
        .param("billingMode", config.billing_mode.clone());
    finish(
        client,
        request,
        MediaKind::Image,
        &config.image_model,
        spec.prompt,
        config,
        "jpg",
    )
    .await
}

async fn generate_video(
    client: &SogniClient,
    spec: VideoSpec,
    config: &PipelineConfig,
    quantity: u32,
    duration_override: Option<f64>,
    aspect_override: Option<&str>,
) -> Result<GeneratedMedia> {
    let duration = duration_override
        .or(config.duration)
        .unwrap_or(10.0)
        .clamp(1.0, 20.0);
    let aspect = aspect_override.unwrap_or(&config.aspect_ratio);
    let (width, height) = video_dimensions(aspect);
    let fps = 24.0;
    let prompt = final_video_prompt(spec);
    confirm_quote(
        client
            .projects
            .estimate_video_cost(&json!({
                "tokenType": config.token_type,
                "model": config.video_model,
                "width": width,
                "height": height,
                "duration": duration,
                "fps": fps,
                "steps": VIDEO_STEPS,
                "numberOfMedia": quantity
            }))
            .await,
        config.assume_yes,
    )?;
    println!(
        "Generating {quantity} video(s) with {} ({duration:.0}s, {fps:.0}fps, {width}x{height})...",
        config.video_model
    );
    let request = ProjectRequest::video(&config.video_model, &prompt)
        .network(Network::Fast)
        .number_of_media(quantity)
        .dimensions(width, height)
        .duration(duration)
        .fps(fps)
        .steps(VIDEO_STEPS)
        .guidance(1.0)
        .param("sampler", "euler")
        .param("scheduler", "simple")
        .param("seed", -1)
        .param("tokenType", config.token_type.clone())
        .param("billingMode", config.billing_mode.clone());
    finish(
        client,
        request,
        MediaKind::Video,
        &config.video_model,
        prompt,
        config,
        "mp4",
    )
    .await
}

async fn generate_audio(
    client: &SogniClient,
    spec: SongSpec,
    config: &PipelineConfig,
    quantity: u32,
) -> Result<GeneratedMedia> {
    confirm_quote(
        client
            .projects
            .estimate_audio_cost(&json!({
                "tokenType": config.token_type,
                "model": config.audio_model,
                "duration": spec.duration,
                "steps": AUDIO_STEPS,
                "numberOfMedia": quantity
            }))
            .await,
        config.assume_yes,
    )?;
    println!(
        "Generating {quantity} audio track(s) with {}...",
        config.audio_model
    );
    let mut request = ProjectRequest::audio(&config.audio_model, &spec.prompt)
        .network(Network::Fast)
        .number_of_media(quantity)
        .duration(spec.duration)
        .steps(AUDIO_STEPS)
        .param("bpm", spec.bpm)
        .param("keyscale", spec.keyscale)
        .param("timesignature", spec.timesignature)
        .param("language", spec.language)
        .param("sampler", "euler")
        .param("scheduler", "simple")
        .param("seed", -1)
        .param("outputFormat", "mp3")
        .param("tokenType", config.token_type.clone())
        .param("billingMode", config.billing_mode.clone());
    if !spec.lyrics.is_empty() {
        request = request.param("lyrics", spec.lyrics);
    }
    finish(
        client,
        request,
        MediaKind::Audio,
        &config.audio_model,
        spec.prompt,
        config,
        "mp3",
    )
    .await
}

async fn finish(
    client: &SogniClient,
    request: ProjectRequest,
    kind: MediaKind,
    model: &str,
    prompt: String,
    config: &PipelineConfig,
    extension: &str,
) -> Result<GeneratedMedia> {
    // The estimate/confirmation happens before this submission. Once accepted,
    // wait for terminal project results before attempting any downloads.
    let project = client.projects.create(request).await?;
    let urls = common::progress::wait_with_progress(&project).await?;
    if urls.is_empty() {
        bail!(
            "{} generation completed without downloadable results",
            kind.label()
        );
    }
    let prefix = format!("{}-{}", kind.label(), common::files::slug(&prompt, 48));
    // Download every result, not merely the first variation returned by the job.
    let files =
        common::files::download_results(&urls, &config.output_dir, &prefix, extension).await?;
    Ok(GeneratedMedia {
        kind,
        model: model.to_owned(),
        prompt,
        files,
    })
}

fn confirm_quote(
    quote: sogni_client::Result<sogni_client::CostEstimate>,
    assume_yes: bool,
) -> Result<()> {
    match quote {
        Ok(quote) => common::cli::confirm_estimate(&quote, assume_yes),
        Err(error) => {
            eprintln!("Cost estimate unavailable: {error}");
            common::cli::require_confirmation("Continue without a cost estimate?", assume_yes)
        }
    }
}

fn final_video_prompt(spec: VideoSpec) -> String {
    let mut prompt = spec.prompt;
    append_once(
        &mut prompt,
        &spec.shot_scale,
        &format!("The framing holds a {} shot.", spec.shot_scale),
    );
    append_once(
        &mut prompt,
        &spec.camera_movement,
        &format!("The camera performs a {}.", spec.camera_movement),
    );
    append_once(
        &mut prompt,
        &spec.style_anchor,
        &format!("{}.", spec.style_anchor),
    );
    append_once(
        &mut prompt,
        &spec.stability_anchor,
        &format!("{}.", spec.stability_anchor),
    );
    prompt
}

fn append_once(prompt: &mut String, needle: &str, sentence: &str) {
    if !needle.is_empty()
        && !prompt
            .to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    {
        if !prompt.ends_with(' ') {
            prompt.push(' ');
        }
        prompt.push_str(sentence);
    }
}

pub fn image_dimensions(name: &str) -> (u32, u32) {
    match name {
        "square_hd" | "square" => (1080, 1080),
        "portrait_4_3" => (1080, 1440),
        "landscape_4_3" => (1440, 1080),
        "landscape_16_9" => (1920, 1080),
        _ => (1080, 1920),
    }
}

pub fn video_dimensions(name: &str) -> (u32, u32) {
    match name {
        "landscape" | "widescreen" => (1920, 1088),
        "square" => (1088, 1088),
        "portrait_4_3" => (832, 1088),
        "landscape_4_3" => (1088, 832),
        _ => (1088, 1920),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_metadata_is_appended_once() {
        let prompt = final_video_prompt(VideoSpec {
            prompt: "A dancer crosses the room with a slow push-in.".into(),
            camera_movement: "slow push-in".into(),
            shot_scale: "medium".into(),
            style_anchor: "grainy documentary".into(),
            stability_anchor: "smooth and stabilised".into(),
        });
        assert_eq!(prompt.matches("slow push-in").count(), 1);
        assert!(prompt.contains("grainy documentary"));
    }
}
