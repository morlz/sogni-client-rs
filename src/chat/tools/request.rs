use serde_json::{Map, Value, json};

use super::super::model_routing::{
    BackboneModelOptions, get_video_defaults, is_edit_image_model,
    resolve_hosted_tool_model_selector, routing_data, select_backbone_model,
};
use crate::{AssetRole, Error, ProjectRequest, Result, WorkloadAttribution};

mod image;
mod media;
mod video;

#[derive(Debug)]
pub(super) struct ToolRequestPlan {
    pub params: Map<String, Value>,
    pub assets: Vec<media::ToolMedia>,
    pub attribution: Option<WorkloadAttribution>,
}

impl ToolRequestPlan {
    fn new(media_type: &str, model_id: &str, args: &Value, options: &Value) -> Result<Self> {
        let count = args
            .get("numberOfVariations")
            .or_else(|| options.get("numberOfMedia"))
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        Ok(Self {
            params: json!({
                "type": media_type, "modelId": model_id,
                "positivePrompt": args.get("prompt").and_then(Value::as_str).unwrap_or(""),
                "numberOfMedia": count.round().clamp(1.0, 16.0) as u32,
            })
            .as_object()
            .unwrap()
            .clone(),
            assets: Vec::new(),
            attribution: options
                .get("attribution")
                .filter(|value| !value.is_null())
                .map(|value| serde_json::from_value(value.clone()).map_err(Error::from))
                .transpose()?,
        })
    }

    fn copy(&mut self, args: &Value, fields: &[(&str, &str)]) {
        for (from, to) in fields {
            if let Some(value) = args.get(from) {
                self.params.insert((*to).into(), value.clone());
            }
        }
    }

    fn asset(&mut self, role: AssetRole, input: &str, media_type: &'static str, remote: bool) {
        let field = match role {
            AssetRole::ReferenceImage => Some("referenceImage"),
            AssetRole::ReferenceImageEnd => Some("referenceImageEnd"),
            AssetRole::ReferenceAudio => Some("referenceAudio"),
            AssetRole::ReferenceAudioIdentity => Some("referenceAudioIdentity"),
            AssetRole::ReferenceVideo => Some("referenceVideo"),
            _ => None,
        };
        if let Some(field) = field {
            self.params.insert(field.into(), json!(true));
        }
        self.assets.push(media::ToolMedia {
            role,
            input: input.into(),
            media_type,
            remote,
        });
    }

    fn dimensions(&mut self, args: &Value, model_id: &str, fps_override: bool) {
        let defaults = get_video_defaults(model_id);
        self.params.insert(
            "width".into(),
            number_or(args, "width", f64::from(defaults.width)),
        );
        self.params.insert(
            "height".into(),
            number_or(args, "height", f64::from(defaults.height)),
        );
        self.params.insert(
            "fps".into(),
            if fps_override {
                number_or(args, "fps", f64::from(defaults.fps))
            } else {
                json!(defaults.fps)
            },
        );
    }

    pub async fn into_request(self, rest: &crate::RestClient) -> Result<ProjectRequest> {
        let mut request = ProjectRequest::from_value(Value::Object(self.params))?;
        for asset in self.assets {
            let source = asset.source(rest).await?;
            request = request.asset(asset.role, source);
        }
        if let Some(attribution) = self.attribution {
            request = request.attribution(attribution);
        }
        Ok(request)
    }
}

pub(super) fn plan_tool_request(
    tool: &str,
    args: &Value,
    options: &Value,
    models: &[Value],
) -> Result<ToolRequestPlan> {
    let mut plan = match tool {
        "generate_image" | "edit_image" => image::plan(tool, args, options, models)?,
        "generate_video" => video::generate(args, options, models)?,
        "sound_to_video" => video::sound(args, options, models)?,
        "video_to_video" => video::transform(args, options, models)?,
        "generate_music" => music(args, options, models)?,
        _ => {
            return Err(Error::InvalidInput(format!(
                "tool {tool} must be executed by hosted chat or a durable run"
            )));
        }
    };
    plan.copy(
        options,
        &[("tokenType", "tokenType"), ("network", "network")],
    );
    Ok(plan)
}

fn select(
    models: &[Value],
    media_type: &str,
    requested: Option<&str>,
    workflows: Option<&[&str]>,
    preferred: &[&str],
    filter: Option<fn(&str) -> bool>,
) -> Result<String> {
    Ok(select_backbone_model(
        models,
        &BackboneModelOptions {
            media_type,
            requested_model: requested,
            workflows,
            preferred_model_ids: preferred,
            filter,
        },
    )?
    .model_id)
}

fn music(args: &Value, options: &Value, models: &[Value]) -> Result<ToolRequestPlan> {
    let requested = resolve_hosted_tool_model_selector("generate_music", args);
    // Preserve source ordering: XL music first; canonical raw requests win.
    let preferred: Vec<_> = [
        "aceStepXlTurbo",
        "aceStepXlSft",
        "aceStepTurbo",
        "aceStepSft",
        "minimaxMusic3",
        "qwen3TtsCustomVoice",
        "qwen3TtsVoiceClone",
        "qwen3TtsVoiceDesign",
    ]
    .iter()
    .filter_map(|key| routing_data()["preferred"]["audio"][key].as_str())
    .collect();
    let model = select(
        models,
        "audio",
        requested.as_deref(),
        None,
        &preferred,
        None,
    )?;
    let mut plan = ToolRequestPlan::new("audio", &model, args, options)?;
    plan.copy(
        args,
        &[
            ("duration", "duration"),
            ("bpm", "bpm"),
            ("keyscale", "keyscale"),
            ("lyrics", "lyrics"),
            ("language", "language"),
            ("output_format", "outputFormat"),
            ("seed", "seed"),
        ],
    );
    if let Some(signature) = args.get("timesignature") {
        let normalized = signature
            .as_str()
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| signature.as_f64().map(|n| (n + 0.5).floor().to_string()));
        if let Some(value) = normalized {
            plan.params.insert("timesignature".into(), json!(value));
        }
    }
    if let Some(value) = args.get("composer_mode").and_then(Value::as_bool) {
        plan.params.insert("composerMode".into(), json!(value));
    }
    for (source, target) in [
        ("prompt_strength", "promptStrength"),
        ("creativity", "creativity"),
    ] {
        if let Some(value) = args.get(source).and_then(Value::as_f64) {
            plan.params.insert(target.into(), json!(value));
        }
    }
    Ok(plan)
}

fn string<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn number_or(args: &Value, key: &str, fallback: f64) -> Value {
    json!(
        args.get(key)
            .and_then(Value::as_f64)
            .filter(|value| *value != 0.0)
            .unwrap_or(fallback)
    )
}

#[cfg(test)]
mod tests;
