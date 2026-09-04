use std::{collections::HashMap, sync::OnceLock, time::Duration};

use serde_json::{Map, Value, json};

use super::api::ChatApi;
use crate::{Error, ProjectRequest, Result};

pub const HOSTED_TOOL_NAMES: &[&str] = &[
    "generate_image",
    "generate_video",
    "generate_music",
    "edit_image",
    "apply_style",
    "restore_photo",
    "upscale_image",
    "refine_result",
    "animate_photo",
    "change_angle",
    "video_to_video",
    "stitch_video",
    "orbit_video",
    "dance_montage",
    "sound_to_video",
    "extend_video",
    "replace_video_segment",
    "overlay_video",
    "add_subtitles",
    "enhance_prompt",
    "compose_lyrics",
    "compose_instrumental",
    "compose_script",
    "compose_workflow",
    "compose_workflow_template",
];

/// Access to the bundled, version-pinned Sogni hosted tool definitions.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostedTools;

impl HostedTools {
    pub fn get(self, name: &str) -> Option<Value> {
        hosted_tool_index().get(name).cloned()
    }

    pub fn all(self) -> Vec<Value> {
        HOSTED_TOOL_NAMES
            .iter()
            .filter_map(|name| self.get(name))
            .collect()
    }
}

#[must_use]
pub fn is_sogni_tool_call(tool_call: &Value) -> bool {
    tool_call
        .pointer("/function/name")
        .and_then(Value::as_str)
        .is_some_and(|name| HOSTED_TOOL_NAMES.contains(&name))
}

#[must_use]
pub fn parse_tool_call_arguments(tool_call: &Value) -> Map<String, Value> {
    tool_call
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

fn hosted_tool_index() -> &'static HashMap<String, Value> {
    static TOOLS: OnceLock<HashMap<String, Value>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        let manifest: Value = serde_json::from_str(include_str!("../../data/hosted_tools.json"))
            .expect("bundled hosted tool manifest must be valid JSON");
        let tools = manifest
            .get("tools")
            .and_then(Value::as_array)
            .expect("bundled hosted tool manifest must contain tools");
        let index = tools
            .iter()
            .filter_map(|tool| {
                let name = tool.pointer("/function/name")?.as_str()?;
                Some((name.to_owned(), tool.clone()))
            })
            .collect::<HashMap<_, _>>();
        assert_eq!(
            index.len(),
            HOSTED_TOOL_NAMES.len(),
            "bundled hosted tool manifest does not match the Rust SDK surface"
        );
        assert!(
            HOSTED_TOOL_NAMES
                .iter()
                .all(|name| index.contains_key(*name)),
            "bundled hosted tool manifest is missing a public tool"
        );
        index
    })
}

impl ChatApi {
    /// Execute one direct Sogni media tool through the Supernet project API.
    pub async fn execute_tool_call(
        &self,
        tool_call: &Value,
        options: Option<&Value>,
    ) -> Result<Value> {
        if !is_sogni_tool_call(tool_call) {
            return Err(Error::InvalidInput("not a Sogni tool call".into()));
        }
        let name = tool_call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !matches!(
            name,
            "generate_image"
                | "edit_image"
                | "generate_video"
                | "sound_to_video"
                | "video_to_video"
                | "generate_music"
        ) {
            return Err(Error::InvalidInput(format!(
                "tool {name} must be executed by hosted chat or a durable run"
            )));
        }
        let args = Value::Object(parse_tool_call_arguments(tool_call));
        let media_type = if name == "generate_music" {
            "audio"
        } else if matches!(name, "generate_video" | "sound_to_video" | "video_to_video") {
            "video"
        } else {
            "image"
        };
        let models = self
            .inner
            .projects
            .wait_for_models(Duration::from_secs(10))
            .await?;
        let requested = args
            .get(if media_type == "video" {
                "videoModel"
            } else {
                "model"
            })
            .and_then(Value::as_str);
        let model_id = requested
            .filter(|requested| {
                models.iter().any(|model| {
                    model.get("id").and_then(Value::as_str) == Some(*requested)
                        && model
                            .get("media")
                            .and_then(Value::as_str)
                            .unwrap_or("image")
                            == media_type
                })
            })
            .map(ToOwned::to_owned)
            .or_else(|| {
                models
                    .iter()
                    .filter(|model| {
                        model
                            .get("media")
                            .and_then(Value::as_str)
                            .unwrap_or("image")
                            == media_type
                    })
                    .max_by_key(|model| {
                        model
                            .get("workerCount")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                    })
                    .and_then(|model| model.get("id").and_then(Value::as_str))
                    .map(ToOwned::to_owned)
            })
            .ok_or_else(|| Error::Protocol(format!("no {media_type} model is available")))?;
        let mut request = ProjectRequest::new(
            media_type,
            model_id.clone(),
            args.get("prompt").and_then(Value::as_str).unwrap_or(""),
        );
        for field in [
            "negativePrompt",
            "duration",
            "width",
            "height",
            "seed",
            "bpm",
            "lyrics",
            "keyscale",
            "outputFormat",
        ] {
            if let Some(value) = args.get(field) {
                request = request.param(field, value.clone());
            }
        }
        if let Some(options) = options {
            for field in ["tokenType", "network"] {
                if let Some(value) = options.get(field) {
                    request = request.param(field, value.clone());
                }
            }
        }
        if media_type == "video" {
            request = request
                .param("fps", args.get("fps").cloned().unwrap_or(json!(24)))
                .param(
                    "duration",
                    args.get("duration").cloned().unwrap_or(json!(5)),
                )
                .param("width", args.get("width").cloned().unwrap_or(json!(768)))
                .param("height", args.get("height").cloned().unwrap_or(json!(512)));
        }
        let project = self.inner.projects.create(request).await?;
        let timeout = options
            .and_then(|options| options.get("timeoutSeconds"))
            .and_then(Value::as_u64)
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(30 * 60));
        let urls = project.wait_for_completion(Some(timeout)).await?;
        Ok(json!({
            "toolCallId": tool_call.get("id"),
            "toolName": name,
            "success": true,
            "resultUrls": urls,
            "content": serde_json::to_string(&json!({
                "success": true,
                "media_type": media_type,
                "urls": urls,
                "model": model_id,
                "prompt": args.get("prompt").and_then(Value::as_str).unwrap_or(""),
            }))?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_manifest_matches_public_names() {
        let tools = HostedTools.all();
        assert_eq!(tools.len(), HOSTED_TOOL_NAMES.len());
        assert!(tools.iter().all(is_sogni_tool_call));
    }

    #[test]
    fn parses_tool_arguments_defensively() {
        let call =
            json!({"function": {"name": "generate_image", "arguments": "{\"prompt\":\"x\"}"}});
        assert_eq!(parse_tool_call_arguments(&call)["prompt"], "x");
        assert!(is_sogni_tool_call(&call));
    }
}
