use serde_json::{Value, json};

pub fn intent_tools() -> Vec<Value> {
    vec![
        intent_tool(
            "generate_image",
            "Generate an image when the user asks to create, draw, or make a picture.",
            json!({
                "prompt": {"type": "string", "description": "The user's raw image request. Do not embellish it."},
                "quantity": {"type": "number", "minimum": 1, "maximum": 512}
            }),
            &[],
        ),
        intent_tool(
            "generate_video",
            "Generate a short video, clip, or animation.",
            json!({
                "prompt": {"type": "string", "description": "The user's raw video request. Do not add camera details."},
                "duration": {"type": "number", "minimum": 1, "maximum": 20},
                "quantity": {"type": "number", "minimum": 1, "maximum": 512},
                "aspect_ratio": {
                    "type": "string",
                    "enum": ["portrait", "landscape", "square", "portrait_4_3", "landscape_4_3", "widescreen"]
                }
            }),
            &[],
        ),
        intent_tool(
            "generate_music",
            "Generate music, a song, beat, or other audio track.",
            json!({
                "prompt": {"type": "string", "description": "The user's raw music request. Do not add production details."},
                "duration": {"type": "number", "minimum": 10, "maximum": 600},
                "quantity": {"type": "number", "minimum": 1, "maximum": 512}
            }),
            &[],
        ),
    ]
}

fn intent_tool(name: &str, description: &str, properties: Value, extra_required: &[&str]) -> Value {
    let mut required = vec![json!("prompt")];
    required.extend(extra_required.iter().map(|value| json!(value)));
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false
            }
        }
    })
}

pub fn image_composer() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "compose_image",
            "description": "Return a production-ready image generation specification.",
            "parameters": {
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "80-180 words of flowing descriptive prose."},
                    "image_size": {
                        "type": "string",
                        "enum": ["square_hd", "portrait_4_3", "portrait_16_9", "landscape_4_3", "landscape_16_9"]
                    }
                },
                "required": ["prompt", "image_size"],
                "additionalProperties": false
            }
        }
    })
}

pub fn video_composer() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "compose_video",
            "description": "Return a production-ready video generation specification.",
            "parameters": {
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "One unbroken paragraph describing one continuous shot."},
                    "camera_movement": {
                        "type": "string",
                        "enum": ["static tripod", "slow push-in", "slow pull-back", "smooth pan left", "smooth pan right", "slow tilt up", "slow tilt down", "slow arc left", "slow arc right", "tracking follow", "handheld subtle drift"]
                    },
                    "shot_scale": {"type": "string", "enum": ["wide", "medium", "close-up"]},
                    "style_anchor": {"type": "string"},
                    "stability_anchor": {"type": "string"}
                },
                "required": ["prompt", "camera_movement", "shot_scale"],
                "additionalProperties": false
            }
        }
    })
}

pub fn song_composer() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "compose_song",
            "description": "Return a complete music generation specification.",
            "parameters": {
                "type": "object",
                "properties": {
                    "positivePrompt": {"type": "string", "description": "Dense producer brief in flowing prose."},
                    "lyrics": {"type": "string", "description": "Lyrics with enriched section headers, or empty for instrumental."},
                    "bpm": {"type": "number", "minimum": 30, "maximum": 300},
                    "keyscale": {"type": "string"},
                    "timesignature": {"type": "string", "enum": ["2", "3", "4", "6"]},
                    "duration": {"type": "number", "minimum": 10, "maximum": 600},
                    "language": {"type": "string"}
                },
                "required": ["positivePrompt", "lyrics", "bpm", "keyscale", "timesignature", "duration", "language"],
                "additionalProperties": false
            }
        }
    })
}
