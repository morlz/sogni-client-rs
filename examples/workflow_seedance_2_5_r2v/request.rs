use serde_json::{Map, Value, json};

use super::{
    config::{Args, TaskType, dimensions, duration, model_config},
    media::MediaUrls,
};

pub fn build_direct_params(args: &Args, urls: &MediaUrls) -> Value {
    let (width, height) = dimensions(&args.resolution).expect("validated resolution");
    let mut params = json!({
        "type": "video", "network": "fast", "tokenType": "spark",
        "modelId": args.model, "positivePrompt": args.prompt(),
        "numberOfMedia": args.number_of_media, "duration": duration(args),
        "fps": 24, "width": width, "height": height,
        "generateAudio": args.generate_audio, "outputFormat": "mp4"
    });
    if !urls.images.is_empty() {
        params["referenceImageUrls"] = json!(urls.images);
    }
    if !urls.videos.is_empty() {
        params["referenceVideoUrls"] = json!(urls.videos);
    }
    if !urls.audios.is_empty() {
        params["referenceAudioUrls"] = json!(urls.audios);
    }
    if model_config(&args.model)
        .expect("validated model")
        .supports_task_type
    {
        // Only Seedance 2.5 understands this direct Projects API discriminator.
        params["seedanceTaskType"] = json!(args.task_type.as_str());
    }
    params
}

pub fn build_creative_agent_request(args: &Args, urls: &MediaUrls) -> Value {
    let selector = model_config(&args.model)
        .expect("validated model")
        .selector
        .expect("validated selector");
    let common = json!({
        "prompt": args.prompt(), "videoModel": selector, "generateAudio": args.generate_audio
    });
    // Creative Agent uses semantic tool names instead of the direct
    // `seedanceTaskType` field.
    let (tool, arguments) = match args.task_type {
        TaskType::Reference => {
            let mut values = object(common);
            values.insert("expandPrompt".into(), json!(true));
            values.insert("duration".into(), json!(duration(args)));
            values.insert(
                "targetResolution".into(),
                json!(resolution_number(&args.resolution)),
            );
            values.insert("numberOfVariations".into(), json!(args.number_of_media));
            add_indices(&mut values, "referenceImageIndices", urls.images.len());
            add_indices(&mut values, "referenceVideoIndices", urls.videos.len());
            add_indices(&mut values, "referenceAudioIndices", urls.audios.len());
            ("generate_video", Value::Object(values))
        }
        TaskType::Edit => {
            let mut values = object(common);
            values.extend([
                ("expandPrompt".into(), json!(true)),
                ("videoSourceIndex".into(), json!(-1)),
                ("controlMode".into(), json!("seedance-v2v")),
                (
                    "targetResolution".into(),
                    json!(resolution_number(&args.resolution)),
                ),
            ]);
            if !urls.images.is_empty() {
                values.insert("sourceImageIndex".into(), json!(-1));
            }
            ("video_to_video", Value::Object(values))
        }
        TaskType::Extend => (
            "extend_video",
            json!({
                "prompt": args.prompt(), "duration": duration(args), "videoIndex": -1,
                "videoModel": selector
            }),
        ),
    };
    json!({
        "tokenType": "spark",
        "mediaReferences": media_references(urls),
        "input": {"title": format!("Seedance {} example", args.task_type.as_str()),
            "steps": [{"id": format!("seedance_{}", args.task_type.as_str()), "toolName": tool, "arguments": arguments}]}
    })
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("JSON object")
}
fn resolution_number(value: &str) -> u32 {
    value.trim_end_matches('p').parse().unwrap_or(720)
}
fn add_indices(target: &mut Map<String, Value>, name: &str, count: usize) {
    if count > 0 {
        // Negative indices address request-scoped media references rather than
        // outputs from earlier workflow steps; ordering is modality-local.
        target.insert(
            name.into(),
            json!((1..=count).map(|n| -(n as i64)).collect::<Vec<_>>()),
        );
    }
}
fn media_references(urls: &MediaUrls) -> Vec<Value> {
    [
        ("image", &urls.images),
        ("video", &urls.videos),
        ("audio", &urls.audios),
    ]
    .into_iter()
    .flat_map(|(kind, values)| {
        values
            .iter()
            .map(move |url| json!({"kind": kind, "url": url}))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::config::{Layer, validate};
    use clap::Parser;

    fn args(extra: &[&str]) -> Args {
        Args::try_parse_from(
            std::iter::once("example")
                .chain(std::iter::once("Contract check prompt."))
                .chain(extra.iter().copied()),
        )
        .unwrap()
    }
    fn args_without_prompt(extra: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("example").chain(extra.iter().copied())).unwrap()
    }
    fn urls(images: usize, videos: usize, audios: usize) -> MediaUrls {
        MediaUrls {
            images: (1..=images)
                .map(|n| format!("https://cdn.example.com/image-{n}.jpg"))
                .collect(),
            videos: (1..=videos)
                .map(|n| format!("https://cdn.example.com/video-{n}.mp4"))
                .collect(),
            audios: (1..=audios)
                .map(|n| format!("https://cdn.example.com/audio-{n}.mp3"))
                .collect(),
        }
    }

    #[test]
    fn direct_seedance_25_contract() {
        let reference = args(&["--audio", "https://cdn.example.com/voice.mp3"]);
        validate(&reference).unwrap();
        let params = build_direct_params(&reference, &urls(0, 0, 1));
        assert_eq!(params["seedanceTaskType"], "reference");
        assert_eq!(
            (
                params["width"].as_u64(),
                params["height"].as_u64(),
                params["fps"].as_u64()
            ),
            (Some(1280), Some(720), Some(24))
        );
        for task in ["edit", "extend"] {
            let mut values = vec![
                "--task-type",
                task,
                "--video",
                "https://cdn.example.com/source.mp4",
            ];
            if task == "edit" {
                values.extend(["--duration", "5"]);
            }
            let options = args(&values);
            validate(&options).unwrap();
            assert_eq!(
                build_direct_params(&options, &urls(0, 1, 0))["seedanceTaskType"],
                task
            );
        }
    }

    #[test]
    fn omitted_prompt_builds_task_specific_direct_and_agent_requests() {
        for (task, extra, media, expected) in [
            (
                "reference",
                vec![
                    "--image",
                    "https://x/image.jpg",
                    "--video",
                    "https://x/video.mp4",
                ],
                urls(1, 1, 0),
                "Use @Image1 for the subject and @Video1 for camera motion in a cohesive new clip.",
            ),
            (
                "edit",
                vec!["--video", "https://x/video.mp4", "--duration", "5"],
                urls(0, 1, 0),
                "Edit @Video1. Change the environment while preserving the subject, timing, and sound.",
            ),
            (
                "extend",
                vec!["--video", "https://x/video.mp4"],
                urls(0, 1, 0),
                "Extend @Video1 after its ending; preserve the cast, scene, pacing, and sound.",
            ),
        ] {
            let options = args_without_prompt(
                &["--task-type", task]
                    .into_iter()
                    .chain(extra)
                    .collect::<Vec<_>>(),
            );
            validate(&options).unwrap();
            assert_eq!(
                build_direct_params(&options, &media)["positivePrompt"],
                expected
            );
            assert_eq!(
                build_creative_agent_request(&options, &media)["input"]["steps"][0]["arguments"]["prompt"],
                expected
            );
        }
    }

    #[test]
    fn rejects_missing_media_duration_and_oversized_resolution() {
        for task in ["edit", "extend"] {
            assert!(validate(&args(&["--task-type", task])).is_err());
        }
        assert!(validate(&args(&[])).is_err());
        assert!(
            validate(&args(&[
                "--task-type",
                "edit",
                "--video",
                "https://x/source.mp4"
            ]))
            .is_err()
        );
        assert!(
            validate(&args(&[
                "--audio",
                "https://x/a.mp3",
                "--resolution",
                "1080p"
            ]))
            .is_err()
        );
    }

    #[test]
    fn limits_and_legacy_contract() {
        let mut owned = vec![
            "example".to_owned(),
            "Contract check prompt.".to_owned(),
            "--duration".into(),
            "30".into(),
        ];
        for (flag, count, ext) in [
            ("--image", 30, "jpg"),
            ("--video", 10, "mp4"),
            ("--audio", 10, "mp3"),
        ] {
            for n in 1..=count {
                owned.extend([flag.into(), format!("https://cdn.example.com/{n}.{ext}")]);
            }
        }
        let maximum = Args::try_parse_from(owned).unwrap();
        validate(&maximum).unwrap();
        let legacy = args(&["--model", "seedance-2-0", "--image", "https://x/i.jpg"]);
        validate(&legacy).unwrap();
        assert!(
            build_direct_params(&legacy, &urls(1, 0, 0))
                .get("seedanceTaskType")
                .is_none()
        );
        let mut duration_30 = legacy_args();
        duration_30.duration = Some(30.0);
        assert!(validate(&duration_30).is_err());
        let mut audio_only = legacy_args();
        audio_only.images.clear();
        audio_only.audios.push("https://x/a.mp3".into());
        assert!(validate(&audio_only).is_err());
        let mut ten_images = legacy_args();
        ten_images.images = (0..10).map(|n| format!("https://x/{n}.jpg")).collect();
        assert!(validate(&ten_images).is_err());
    }

    #[test]
    fn creative_agent_maps_each_task_without_direct_field() {
        for (task, tool) in [
            ("reference", "generate_video"),
            ("edit", "video_to_video"),
            ("extend", "extend_video"),
        ] {
            let media_flag = if task == "reference" {
                "--audio"
            } else {
                "--video"
            };
            let options = args(&[
                "--layer",
                "creative-agent",
                "--task-type",
                task,
                media_flag,
                "https://x/media.mp4",
            ]);
            validate(&options).unwrap();
            let refs = if task == "reference" {
                urls(0, 0, 1)
            } else {
                urls(0, 1, 0)
            };
            let request = build_creative_agent_request(&options, &refs);
            assert_eq!(request["input"]["steps"][0]["toolName"], tool);
            let text = request.to_string();
            assert!(!text.contains("seedanceTaskType") && !text.contains("seedance_task_type"));
        }
    }

    fn legacy_args() -> Args {
        let mut value = args(&["--model", "seedance-2-0", "--image", "https://x/i.jpg"]);
        value.layer = Layer::Direct;
        value
    }
}
