use std::{collections::HashMap, sync::OnceLock, time::Duration};

use serde_json::{Map, Value, json};

use super::api::ChatApi;
use crate::{Error, Result};

pub const HOSTED_TOOL_NAMES: &[&str] = &[
    "generate_image",
    "generate_video",
    "generate_music",
    "generate_speech",
    "edit_image",
    "apply_style",
    "restore_photo",
    "upscale_image",
    "upscale_video",
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

mod request;
mod wait;

impl ChatApi {
    /// Execute one of the six direct media tools using upstream model routing.
    /// Other hosted tools execute through hosted chat or durable chat runs.
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
        let empty_options = Value::Null;
        let options = options.unwrap_or(&empty_options);
        let models = self
            .inner
            .projects
            .wait_for_models(Duration::from_secs(10))
            .await?;
        let plan = request::plan_tool_request(name, &args, options, &models)?;
        let model_id = plan.params["modelId"].clone();
        let media_type = plan.params["type"].clone();
        let request = plan.into_request(&self.inner.client.rest).await?;
        let project = self.inner.projects.create(request).await?;
        let timeout = options
            .get("timeoutSeconds")
            .and_then(Value::as_u64)
            .map(Duration::from_secs)
            .or_else(|| {
                options
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .map(Duration::from_millis)
            })
            .unwrap_or(Duration::from_secs(90 * 60));
        let urls = match wait::wait_for_tool_project(&project, timeout).await {
            Ok(urls) => urls,
            Err(error) => {
                let _ = project.cancel().await;
                return Err(error);
            }
        };
        let mut content = json!({
            "success": true, "media_type": media_type, "urls": urls, "model": model_id,
            "prompt": args.get("prompt").and_then(Value::as_str).unwrap_or(""),
        });
        let last_frames: Vec<_> = project
            .jobs()
            .iter()
            .map(|job| job.last_frame_url())
            .collect();
        if last_frames.iter().any(Option::is_some) {
            content["lastFrameUrls"] = json!(last_frames);
        }
        Ok(json!({
            "toolCallId": tool_call.get("id"), "toolName": name,
            "success": true, "resultUrls": urls, "content": serde_json::to_string(&content)?,
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
