use super::{
    config::{Args, Mode},
    media::Media,
};
use serde_json::{Map, Value, json};
pub fn tool_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Ia2v => "sound_to_video",
        Mode::V2v => "video_to_video",
        _ => "generate_video",
    }
}
pub fn tool_arguments(args: &Args, media: &Media) -> Value {
    let (width, height) = args.dimensions().expect("validated dimensions");
    let mut value = json!({"prompt":args.prompt,"expand_prompt":!args.no_expand_prompt,"model":args.model_id().expect("validated model"),"duration":args.duration,"width":width,"height":height,"number_of_variations":args.number});
    if let Some(seed) = args.seed {
        value["seed"] = json!(seed);
    }
    if let Some(audio) = args.generate_audio() {
        value["generate_audio"] = json!(audio);
    }
    match args.mode() {
        Mode::T2v => {
            value["fps"] = json!(24);
            arrays(&mut value, media);
            if let Some(url) = &media.identity_audio {
                value["reference_audio_identity_url"] = json!(url);
            }
            strength(args, &mut value);
        }
        Mode::I2v => {
            value["fps"] = json!(24);
            primary(
                &mut value,
                "reference_image_url",
                "reference_image_urls",
                &media.images,
            );
            if let Some(url) = &media.end_image {
                value["reference_image_end_url"] = json!(url);
                value["first_frame_strength"] = json!(args.first_frame_strength.unwrap_or(1.0));
                value["last_frame_strength"] = json!(args.last_frame_strength.unwrap_or(1.0));
            }
            if !media.videos.is_empty() {
                value["reference_video_urls"] = json!(media.videos);
            }
            if !media.audios.is_empty() {
                value["reference_audio_urls"] = json!(media.audios);
            }
            if let Some(url) = &media.identity_audio {
                value["reference_audio_identity_url"] = json!(url);
            }
            strength(args, &mut value);
        }
        Mode::Ia2v => {
            primary(
                &mut value,
                "reference_image_url",
                "reference_image_urls",
                &media.images,
            );
            primary(
                &mut value,
                "reference_audio_url",
                "reference_audio_urls",
                &media.audios,
            );
            if !media.videos.is_empty() {
                value["reference_video_urls"] = json!(media.videos);
            }
            if let Some(start) = args.audio_start {
                value["audio_start"] = json!(start);
            }
        }
        Mode::V2v => {
            primary(
                &mut value,
                "reference_video_url",
                "reference_video_urls",
                &media.videos,
            );
            value["control_mode"] = json!(args.control_mode.as_deref().unwrap_or("seedance-v2v"));
            primary(
                &mut value,
                "reference_image_url",
                "reference_image_urls",
                &media.images,
            );
            if !media.audios.is_empty() {
                value["reference_audio_urls"] = json!(media.audios);
            }
            if let Some(start) = args.video_start {
                value["video_start"] = json!(start);
            }
        }
    }
    value
}
fn arrays(value: &mut Value, media: &Media) {
    if !media.images.is_empty() {
        value["reference_image_urls"] = json!(media.images);
    }
    if !media.videos.is_empty() {
        value["reference_video_urls"] = json!(media.videos);
    }
    if !media.audios.is_empty() {
        value["reference_audio_urls"] = json!(media.audios);
    }
}
fn primary(value: &mut Value, one: &str, many: &str, urls: &[String]) {
    if let Some(first) = urls.first() {
        value[one] = json!(first);
        if urls.len() > 1 {
            value[many] = json!(&urls[1..]);
        }
    }
}
fn strength(args: &Args, value: &mut Value) {
    if let Some(v) = args.audio_identity_strength {
        value["audio_identity_strength"] = json!(v);
    }
}
fn negatives(count: usize) -> Vec<i64> {
    (1..=count).map(|v| -(v as i64)).collect()
}
pub fn workflow(args: &Args, tool_args: &Value) -> Value {
    let mode = args.mode();
    let mut images = Vec::new();
    let mut videos = Vec::new();
    let mut audios = Vec::new();
    push_one(tool_args, "reference_image_url", &mut images);
    push_one(tool_args, "reference_image_end_url", &mut images);
    push_one(tool_args, "reference_video_url", &mut videos);
    push_one(tool_args, "reference_audio_url", &mut audios);
    push_many(tool_args, "reference_image_urls", &mut images);
    push_many(tool_args, "reference_video_urls", &mut videos);
    push_many(tool_args, "reference_audio_urls", &mut audios);
    let mut fields = object(
        json!({"prompt":tool_args["prompt"],"expandPrompt":tool_args["expand_prompt"],"videoModel":args.selector().expect("selector"),"duration":tool_args["duration"],"numberOfVariations":tool_args["number_of_variations"]}),
    );
    if let Some(value) = tool_args.get("generate_audio") {
        fields.insert("generateAudio".into(), value.clone());
    }
    match mode {
        Mode::T2v | Mode::I2v => {
            fields.insert("width".into(), tool_args["width"].clone());
            fields.insert("height".into(), tool_args["height"].clone());
            insert_indices(&mut fields, "referenceImageIndices", images.len());
            insert_indices(&mut fields, "referenceVideoIndices", videos.len());
            insert_indices(&mut fields, "referenceAudioIndices", audios.len());
            if mode == Mode::I2v && !images.is_empty() && !args.prompt.contains("@Image1") {
                fields.insert(
                    "prompt".into(),
                    json!(format!(
                        "Use @Image1 as the opening shot reference. {}",
                        args.prompt
                    )),
                );
            }
        }
        Mode::Ia2v => {
            fields.insert("sourceImageIndex".into(), json!(-1));
            fields.insert("audioSourceIndex".into(), json!(-1));
            if let Some(v) = args.audio_start {
                fields.insert("audioStart".into(), json!(v));
            }
        }
        Mode::V2v => {
            fields.insert("videoSourceIndex".into(), json!(-1));
            if !images.is_empty() {
                fields.insert("sourceImageIndex".into(), json!(-1));
            }
            fields.insert("controlMode".into(), json!("seedance-v2v"));
        }
    }
    let refs = images
        .iter()
        .map(|u| json!({"kind":"image","url":u}))
        .chain(videos.iter().map(|u| json!({"kind":"video","url":u})))
        .chain(audios.iter().map(|u| json!({"kind":"audio","url":u})))
        .collect::<Vec<_>>();
    json!({"mediaReferences":refs,"input":{"title":format!("Seedance {} example",mode.as_str().to_uppercase()),"steps":[{"id":format!("seedance_{}",mode.as_str()),"toolName":tool_name(mode),"arguments":fields}]}})
}
fn object(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap()
}
fn insert_indices(v: &mut Map<String, Value>, key: &str, count: usize) {
    if count > 0 {
        v.insert(key.into(), json!(negatives(count)));
    }
}
fn push_one(source: &Value, key: &str, target: &mut Vec<String>) {
    if let Some(value) = source.get(key).and_then(Value::as_str) {
        target.push(value.to_owned());
    }
}
fn push_many(source: &Value, key: &str, target: &mut Vec<String>) {
    if let Some(values) = source.get(key).and_then(Value::as_array) {
        target.extend(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned),
        );
    }
}
pub fn chat(args: &Args, tool_args: &Value) -> Value {
    let tool = tool_name(args.mode());
    json!({"model":args.llm_model,"messages":[{"role":"system","content":"You are a precise Sogni media production router. Call the requested tool with the exact arguments."},{"role":"user","content":format!("Create exactly one Seedance {} job by calling {tool}. Do not ask questions. Tool arguments:\n{}",args.mode().as_str().to_uppercase(),serde_json::to_string_pretty(tool_args).unwrap())}],"temperature":0.1,"max_tokens":1600,"think":false,"tokenType":args.token_type.as_str(),"billingMode":args.billing_mode.as_str(),"sogniTools":true,"sogniToolExecution":true,"tools":[{"type":"function","function":{"name":tool,"description":"Execute the requested Seedance video operation.","parameters":{"type":"object","additionalProperties":true,"required":["prompt"],"properties":{"prompt":{"type":"string"}}}}}],"toolChoice":{"type":"function","function":{"name":tool}}})
}
