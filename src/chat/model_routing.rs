use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error, Result,
    utils::{
        get_video_workflow_type, is_gpt_image_model, is_happyhorse_model, is_ltx_model,
        is_minimax_h3_model, is_seedance_model, is_wan3_model,
    },
};

/// Constraints for local media-tool model selection, in upstream priority order.
#[derive(Clone, Copy, Debug)]
pub struct BackboneModelOptions<'a> {
    pub media_type: &'a str,
    pub requested_model: Option<&'a str>,
    pub workflows: Option<&'a [&'a str]>,
    pub preferred_model_ids: &'a [&'a str],
    pub filter: Option<fn(&str) -> bool>,
}

impl<'a> BackboneModelOptions<'a> {
    #[must_use]
    pub fn new(media_type: &'a str) -> Self {
        Self {
            media_type,
            requested_model: None,
            workflows: None,
            preferred_model_ids: &[],
            filter: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SelectedBackboneModel {
    pub model_id: String,
    pub model: Value,
    pub selected_by: BackboneModelSelectionReason,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackboneModelSelectionReason {
    RequestedModel,
    PreferredModel,
    WorkerCount,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoDefaults {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

pub(super) fn routing_data() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/chat_model_routing.json"))
            .expect("bundled upstream chat routing data must be valid")
    })
}

/// Resolve a public hosted-tool selector; unknown raw model IDs pass through.
#[must_use]
pub fn resolve_hosted_tool_model_selector(tool_name: &str, args: &Value) -> Option<String> {
    let video = matches!(
        tool_name,
        "generate_video" | "animate_photo" | "sound_to_video" | "video_to_video"
    );
    let requested = args
        .get(if video { "videoModel" } else { "model" })?
        .as_str()?;
    if requested.trim().is_empty() {
        return None;
    }
    let table = match tool_name {
        "generate_image" => "image",
        "edit_image" => "edit",
        "generate_video" => {
            if args
                .get("referenceImageIndices")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
            {
                "imageVideo"
            } else {
                "textVideo"
            }
        }
        "animate_photo" => "imageVideo",
        "sound_to_video" => "soundToVideo",
        "video_to_video" => "videoToVideo",
        _ => return Some(requested.into()),
    };
    let mut normalized = String::new();
    let mut separator = false;
    for character in requested.trim().to_lowercase().chars() {
        if character == '_' || character.is_whitespace() {
            if !separator {
                normalized.push('-');
            }
            separator = true;
        } else {
            normalized.push(character);
            separator = false;
        }
    }
    let selectors = &routing_data()["selectors"][table];
    let resolved = selectors
        .get(requested)
        .or_else(|| selectors.get(&normalized))
        .and_then(Value::as_str)
        .unwrap_or(requested);
    if tool_name == "video_to_video" && !compatible_workflows(resolved).contains(&"v2v") {
        return None;
    }
    Some(resolved.into())
}

pub(super) fn compatible_workflows(model_id: &str) -> Vec<&'static str> {
    if is_wan3_model(model_id) {
        vec!["t2v", "i2v", "flf2v", "r2v", "a2v", "ia2v"]
    } else if model_id == "seedance-2-5" {
        vec!["t2v", "i2v", "flf2v", "r2v", "ia2v", "v2v"]
    } else if matches!(
        model_id,
        "seedance-2-0" | "seedance-2-0-mini" | "seedance-2-0-fast"
    ) {
        vec!["t2v", "i2v", "ia2v", "v2v"]
    } else {
        get_video_workflow_type(model_id).into_iter().collect()
    }
}

#[must_use]
pub fn filter_video_models_by_workflow(models: &[Value], workflows: &[&str]) -> Vec<String> {
    models
        .iter()
        .filter(|model| model.get("media").and_then(Value::as_str) == Some("video"))
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .filter(|id| {
            compatible_workflows(id)
                .iter()
                .any(|workflow| workflows.contains(workflow))
        })
        .map(ToOwned::to_owned)
        .collect()
}

#[must_use]
pub fn is_edit_image_model(model_id: &str) -> bool {
    is_gpt_image_model(model_id)
        || matches!(
            model_id,
            "qwen_image_edit_2511_fp8"
                | "qwen_image_edit_2511_fp8_lightning"
                | "krea2_identity_edit_v1_2"
                | "dark_beast_krea2_identity_edit_v1_2"
        )
}

#[must_use]
pub fn get_video_defaults(model_id: &str) -> VideoDefaults {
    let (width, height, fps) = if is_wan3_model(model_id) {
        (1920, 1080, 30)
    } else if is_minimax_h3_model(model_id) {
        if model_id == "minimax-h3-ref2va-fp8_r2v_turbo" {
            (960, 544, 24)
        } else {
            (1344, 768, 24)
        }
    } else if matches!(
        get_video_workflow_type(model_id),
        Some("s2v" | "animate-move" | "animate-replace")
    ) {
        (832, 480, 16)
    } else if matches!(
        model_id,
        "seedance-2-0-mini" | "seedance-2-0-fast" | "seedance-2-5"
    ) {
        (1280, 720, 24)
    } else if is_seedance_model(model_id) || is_happyhorse_model(model_id) {
        (1920, 1080, 24)
    } else if is_ltx_model(model_id) {
        (1920, 1088, 24)
    } else {
        (848, 480, 16)
    };
    VideoDefaults { width, height, fps }
}

/// Prefer a compatible requested model, then declared preferences, then workers.
/// Equal worker counts retain catalog order, matching the TypeScript SDK.
pub fn select_backbone_model(
    models: &[Value],
    options: &BackboneModelOptions<'_>,
) -> Result<SelectedBackboneModel> {
    let by_media: Vec<_> = models
        .iter()
        .filter(|model| model.get("media").and_then(Value::as_str) == Some(options.media_type))
        .collect();
    if by_media.is_empty() {
        return Err(Error::InvalidInput(format!(
            "No {} models currently available on the network",
            options.media_type
        )));
    }
    let compatible: Vec<_> = by_media
        .into_iter()
        .filter(|model| {
            let id = model.get("id").and_then(Value::as_str).unwrap_or("");
            options.filter.is_none_or(|filter| filter(id))
                && options.workflows.is_none_or(|workflows| {
                    compatible_workflows(id)
                        .iter()
                        .any(|workflow| workflows.contains(workflow))
                })
        })
        .collect();
    let result = |model: &Value, reason| SelectedBackboneModel {
        model_id: model["id"].as_str().unwrap_or("").into(),
        model: model.clone(),
        selected_by: reason,
    };
    if let Some(requested) = options.requested_model {
        if let Some(model) = compatible.iter().find(|model| model["id"] == requested) {
            return Ok(result(model, BackboneModelSelectionReason::RequestedModel));
        }
    }
    if compatible.is_empty() {
        return Err(Error::InvalidInput(match options.workflows {
            Some(workflows) => format!(
                "No compatible {} models available for workflows: {}",
                options.media_type,
                workflows.join(", ")
            ),
            None => format!(
                "No compatible {} models currently available on the network",
                options.media_type
            ),
        }));
    }
    for id in options.preferred_model_ids {
        if let Some(model) = most_workers(
            compatible
                .iter()
                .copied()
                .filter(|model| model["id"] == *id),
        ) {
            return Ok(result(model, BackboneModelSelectionReason::PreferredModel));
        }
    }
    Ok(result(
        most_workers(compatible).expect("nonempty compatible models"),
        BackboneModelSelectionReason::WorkerCount,
    ))
}

fn most_workers<'a>(models: impl IntoIterator<Item = &'a Value>) -> Option<&'a Value> {
    models.into_iter().reduce(|left, right| {
        let workers = |model: &Value| {
            model
                .get("workerCount")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        if workers(right) > workers(left) {
            right
        } else {
            left
        }
    })
}

#[cfg(test)]
mod tests;
