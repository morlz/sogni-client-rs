use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use sogni_client::SogniClient;

use crate::{common, shared};

use super::{
    schemas,
    types::{ImageSpec, MediaKind, MediaSpec, SongSpec, VideoSpec},
};

const IMAGE_SYSTEM: &str = r#"You are a prompt engineer for text-to-image generation. Call compose_image with 80-180 words of flowing, positive prose. Front-load a concrete subject and action, then environment, composition, named lighting, optional camera/lens, at most two style anchors, and concrete quality details. Never emit a tag list. Choose square_hd for centered products, portrait_4_3 for people, portrait_16_9 for full-body/tall compositions, landscape_4_3 for groups/scenes, and landscape_16_9 for panoramic scenes."#;

const VIDEO_SYSTEM: &str = r#"You are a cinematographer writing for LTX-2.3. Call compose_video with one unbroken paragraph of 4-8 present-tense sentences describing one continuous shot. Establish shot scale and visual language; describe environment, light sources, stable character identity, and one evolving physical action. Attribute dialogue inline with speaker, delivery, and action. Weave ambient sound into prose. Use visible physical cues for emotion. Use positive, concrete language and no lists, cuts, montage, on-screen text, logos, or structural markup."#;

const AUDIO_SYSTEM: &str = r#"You are an expert music producer. Call compose_song with a dense producer brief describing genre, every instrument's role, texture and behavior, vocal character, arrangement arc, and production aesthetic. Put BPM and key only in their dedicated fields. Lyrics use enriched headers such as [Verse 1 - Soft male vocal and arpeggiated guitar]; return an empty lyric string for instrumentals. Choose a musically suitable BPM, major/minor key, time signature, duration, and ISO language code."#;

pub async fn compose(
    client: &SogniClient,
    kind: MediaKind,
    intent: &str,
    duration: f64,
    settings: &shared::ChatSettings,
    assume_yes: bool,
    show_thinking: bool,
) -> Result<MediaSpec> {
    let (label, system, tool) = match kind {
        MediaKind::Image => ("image prompt", IMAGE_SYSTEM, schemas::image_composer()),
        MediaKind::Video => ("video prompt", VIDEO_SYSTEM, schemas::video_composer()),
        MediaKind::Audio => ("song composition", AUDIO_SYSTEM, schemas::song_composer()),
    };
    let user = if kind == MediaKind::Video {
        format!("{intent}\n\n{}", pacing_hint(duration))
    } else {
        intent.to_owned()
    };
    let messages = vec![
        json!({"role": "system", "content": system}),
        json!({"role": "user", "content": user}),
    ];
    shared::estimate_and_print(client, settings, &messages).await;
    common::cli::require_confirmation(&format!("Proceed with paid {label} LLM call?"), assume_yes)?;
    println!("\nComposing {label}...");
    let arguments = stream_required_tool(client, settings, messages, tool, show_thinking).await?;
    match kind {
        MediaKind::Image => Ok(MediaSpec::Image(parse_image(&arguments, intent))),
        MediaKind::Video => Ok(MediaSpec::Video(parse_video(&arguments, intent))),
        MediaKind::Audio => Ok(MediaSpec::Audio(parse_song(&arguments, intent))),
    }
}

async fn stream_required_tool(
    client: &SogniClient,
    settings: &shared::ChatSettings,
    messages: Vec<Value>,
    tool: Value,
    show_thinking: bool,
) -> Result<Map<String, Value>> {
    let mut options = settings.clone();
    options.think = false;
    options.task_profile = "reasoning".into();
    for attempt in 1..=3 {
        let mut request = shared::runtime::request(&options, &messages, true);
        request["tools"] = json!([tool]);
        request["tool_choice"] = json!("required");
        let response = shared::stream_response(client, &request, show_thinking).await;
        match response {
            Ok((_, completion, _)) => {
                let call = completion
                    .tool_calls
                    .first()
                    .context("composition model returned no required tool call")?;
                return serde_json::from_str(&call.function.arguments)
                    .context("parse composition tool arguments");
            }
            Err(error)
                if attempt < 3 && error.to_string().to_ascii_lowercase().contains("timed out") =>
            {
                eprintln!("Composition attempt {attempt}/3 timed out; retrying.");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(error) => return Err(error),
        }
    }
    bail!("composition failed after three attempts")
}

fn parse_image(args: &Map<String, Value>, fallback: &str) -> ImageSpec {
    let prompt = text(args, "prompt").unwrap_or_else(|| fallback.to_owned());
    let image_size = text(args, "image_size")
        .filter(|value| {
            matches!(
                value.as_str(),
                "square_hd" | "portrait_4_3" | "portrait_16_9" | "landscape_4_3" | "landscape_16_9"
            )
        })
        .unwrap_or_else(|| "portrait_16_9".into());
    println!("  Size:   {image_size}");
    println!("  Prompt: {prompt}");
    ImageSpec { prompt, image_size }
}

fn parse_video(args: &Map<String, Value>, fallback: &str) -> VideoSpec {
    let prompt = text(args, "prompt")
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_owned());
    let camera_movement = enum_text(
        args,
        "camera_movement",
        &[
            "static tripod",
            "slow push-in",
            "slow pull-back",
            "smooth pan left",
            "smooth pan right",
            "slow tilt up",
            "slow tilt down",
            "slow arc left",
            "slow arc right",
            "tracking follow",
            "handheld subtle drift",
        ],
        "slow push-in",
    );
    let shot_scale = enum_text(
        args,
        "shot_scale",
        &["wide", "medium", "close-up"],
        "medium",
    );
    let style_anchor = text(args, "style_anchor").unwrap_or_default();
    let stability_anchor =
        text(args, "stability_anchor").unwrap_or_else(|| "smooth and stabilised".into());
    println!("  Camera: {camera_movement}");
    println!("  Scale:  {shot_scale}");
    println!("  Prompt: {prompt}");
    VideoSpec {
        prompt,
        camera_movement,
        shot_scale,
        style_anchor,
        stability_anchor,
    }
}

fn parse_song(args: &Map<String, Value>, fallback: &str) -> SongSpec {
    let prompt = text(args, "positivePrompt").unwrap_or_else(|| fallback.to_owned());
    let lyrics = text(args, "lyrics").unwrap_or_default();
    let bpm = number(args, "bpm")
        .unwrap_or(120.0)
        .round()
        .clamp(30.0, 300.0) as u32;
    let keyscale = normalize_keyscale(&text(args, "keyscale").unwrap_or_else(|| "C major".into()));
    let timesignature = enum_text(args, "timesignature", &["2", "3", "4", "6"], "4");
    let duration = number(args, "duration").unwrap_or(30.0).clamp(10.0, 600.0);
    let language = text(args, "language").unwrap_or_else(|| "en".into());
    println!("  Style:    {prompt}");
    println!("  BPM/key:  {bpm}, {keyscale}");
    println!("  Duration: {duration:.0}s; language {language}");
    if !lyrics.is_empty() {
        println!(
            "  Lyrics:   {} non-empty lines",
            lyrics
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        );
    }
    SongSpec {
        prompt,
        lyrics,
        bpm,
        keyscale,
        timesignature,
        duration,
        language,
    }
}

fn pacing_hint(duration: f64) -> String {
    let actions = (duration / 4.0).round().clamp(1.0, 10.0) as usize;
    if actions == 1 {
        return format!(
            "This clip is {duration:.0} seconds. Write exactly one action and stop after that single moment."
        );
    }
    format!(
        "This clip is {duration:.0} seconds. Write exactly {actions} distinct actions, about {:.0} seconds each. Add no setup or resolution beyond them and stop after action {actions}.",
        duration / actions as f64
    )
}

fn text(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn number(args: &Map<String, Value>, key: &str) -> Option<f64> {
    args.get(key)
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
        .filter(|value| value.is_finite())
}

fn enum_text(args: &Map<String, Value>, key: &str, allowed: &[&str], fallback: &str) -> String {
    text(args, key)
        .filter(|value| allowed.contains(&value.as_str()))
        .unwrap_or_else(|| fallback.into())
}

fn normalize_keyscale(value: &str) -> String {
    let mut parts = value.split_whitespace().collect::<Vec<_>>();
    let Some(scale) = parts.pop() else {
        return "C major".into();
    };
    if parts.is_empty() {
        return value.to_owned();
    }
    format!("{} {}", parts.join(" "), scale.to_ascii_lowercase())
}
