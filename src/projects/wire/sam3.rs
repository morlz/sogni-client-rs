use super::*;

pub(in crate::projects) const SAM3_MODEL_ID: &str = "sam3_image_segment_bf16";
#[cfg(test)]
pub(in crate::projects) const PIXAL3D_MODEL_ID: &str = "pixal3d_int8_i23d";

/// Normalize before constructing the project so its expected result count and
/// MIME type agree with the wire request and a single mask settles the project.
pub(in crate::projects) fn normalize_utility_params(params: &mut Map<String, Value>) {
    if params.get("type").and_then(Value::as_str) == Some("image")
        && params
            .get("modelId")
            .and_then(Value::as_str)
            .is_some_and(is_segmentation_model)
    {
        params.insert("numberOfMedia".into(), json!(1));
        params.insert("numberOfPreviews".into(), json!(0));
        params.insert("outputFormat".into(), json!("png"));
    }
    if params.get("type").and_then(Value::as_str) == Some("image")
        && params
            .get("modelId")
            .and_then(Value::as_str)
            .is_some_and(is_model_artifact_model)
    {
        params.insert("numberOfPreviews".into(), json!(0));
    }
}

pub(super) fn normalize_sam3_prompt(prompt: &Value) -> Result<Value> {
    let prompt = prompt
        .as_object()
        .ok_or_else(|| invalid("sam3Prompt must be an object"))?;
    let unknown = unknown_keys(
        prompt,
        &[
            "points",
            "boxes",
            "text",
            "threshold",
            "multimask",
            "applyMask",
            "maxInstances",
        ],
    );
    if !unknown.is_empty() {
        return Err(invalid(format!(
            "sam3Prompt contains unsupported fields: {}",
            unknown.join(", ")
        )));
    }
    let points = entries(prompt.get("points"), "points", 32)?;
    let mut normalized_points = Vec::new();
    for (index, point) in points.iter().enumerate() {
        let field = format!("sam3Prompt.points[{index}]");
        let point = point
            .as_object()
            .ok_or_else(|| invalid(format!("{field} must be an object")))?;
        if !unknown_keys(point, &["x", "y", "label"]).is_empty() {
            return Err(invalid(format!("{field} contains unsupported fields")));
        }
        let label = point.get("label").and_then(Value::as_str);
        if !matches!(label, Some("positive" | "negative")) {
            return Err(invalid(format!(
                "{field}.label must be \"positive\" or \"negative\""
            )));
        }
        normalized_points.push(json!({
            "x": coordinate(point.get("x"), &format!("{field}.x"))?,
            "y": coordinate(point.get("y"), &format!("{field}.y"))?,
            "label": label,
        }));
    }
    let boxes = entries(prompt.get("boxes"), "boxes", 16)?;
    let mut normalized_boxes = Vec::new();
    for (index, selection) in boxes.iter().enumerate() {
        let field = format!("sam3Prompt.boxes[{index}]");
        let selection = selection
            .as_object()
            .ok_or_else(|| invalid(format!("{field} must be an object")))?;
        if !unknown_keys(selection, &["x0", "y0", "x1", "y1", "label"]).is_empty() {
            return Err(invalid(format!("{field} contains unsupported fields")));
        }
        let x0 = coordinate(selection.get("x0"), &format!("{field}.x0"))?;
        let y0 = coordinate(selection.get("y0"), &format!("{field}.y0"))?;
        let x1 = coordinate(selection.get("x1"), &format!("{field}.x1"))?;
        let y1 = coordinate(selection.get("y1"), &format!("{field}.y1"))?;
        if x0 >= x1 || y0 >= y1 {
            return Err(invalid(format!("{field} must have x0 < x1 and y0 < y1")));
        }
        let label = selection.get("label").map(|value| value.as_str());
        if !matches!(label, None | Some(Some("positive" | "negative"))) {
            return Err(invalid(format!(
                "{field}.label must be \"positive\" or \"negative\""
            )));
        }
        normalized_boxes.push(json!({"x0":x0, "y0":y0, "x1":x1, "y1":y1,
            "label": label.flatten().unwrap_or("positive")}));
    }
    let text = prompt
        .get("text")
        .map(|value| {
            let text = value
                .as_str()
                .ok_or_else(|| invalid("sam3Prompt.text must be a string"))?
                .trim();
            // Match JavaScript string length, including surrogate pairs.
            if text.is_empty() || text.encode_utf16().count() > 240 {
                return Err(invalid("sam3Prompt.text must contain 1 to 240 characters"));
            }
            Ok(text)
        })
        .transpose()?;
    if points.is_empty() && boxes.is_empty() && text.is_none() {
        return Err(invalid(
            "sam3Prompt requires at least one point, box, or text prompt",
        ));
    }
    if text.is_some() && !points.is_empty() {
        return Err(invalid("sam3Prompt cannot combine text and point prompts"));
    }
    if !points.is_empty() && boxes.len() > 1 {
        return Err(invalid(
            "sam3Prompt supports at most one box when point prompts are present",
        ));
    }
    if !points.is_empty()
        && normalized_boxes
            .iter()
            .any(|selection| selection["label"] == "negative")
    {
        return Err(invalid("sam3Prompt negative boxes require a text prompt"));
    }
    let threshold = prompt
        .get("threshold")
        .map(|v| {
            v.as_f64()
                .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                .ok_or_else(|| invalid("sam3Prompt.threshold must be a finite number from 0 to 1"))
        })
        .transpose()?
        .unwrap_or(0.5);
    let multimask = prompt
        .get("multimask")
        .map(|v| {
            v.as_bool()
                .ok_or_else(|| invalid("sam3Prompt.multimask must be a boolean"))
        })
        .transpose()?
        .unwrap_or(!points.is_empty());
    // Multimask chooses click candidates. Explicit false on text/box paths is
    // harmless and omitted; true without points names a caller error.
    if multimask && points.is_empty() {
        return Err(invalid("sam3Prompt.multimask requires point prompts"));
    }
    let apply_mask = prompt
        .get("applyMask")
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| invalid("sam3Prompt.applyMask must be a boolean"))
        })
        .transpose()?
        .unwrap_or(false);
    let max_instances = prompt
        .get("maxInstances")
        .map(|value| {
            value
                .as_u64()
                .filter(|value| (1..=16).contains(value))
                .ok_or_else(|| invalid("sam3Prompt.maxInstances must be an integer from 1 to 16"))
        })
        .transpose()?;
    let mut normalized = json!({
        "points": normalized_points, "boxes": normalized_boxes,
        "threshold": threshold, "applyMask": apply_mask,
    });
    if !points.is_empty() {
        normalized["multimask"] = json!(multimask);
    }
    if let Some(count) = max_instances {
        normalized["maxInstances"] = json!(count);
    }
    if let Some(text) = text {
        normalized["text"] = json!(text);
    }
    Ok(normalized)
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidInput(message.into())
}

fn unknown_keys<'a>(value: &'a Map<String, Value>, allowed: &[&str]) -> Vec<&'a str> {
    value
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .map(String::as_str)
        .collect()
}

fn entries<'a>(value: Option<&'a Value>, name: &str, max: usize) -> Result<&'a [Value]> {
    // Optional arrays use the same empty defaults as the upstream serializer.
    match value.filter(|v| !is_falsy(v)) {
        None => Ok(&[]),
        Some(Value::Array(values)) if values.len() <= max => Ok(values),
        _ => Err(invalid(format!(
            "sam3Prompt.{name} must contain at most {max} entries"
        ))),
    }
}

pub(super) fn is_falsy(value: &Value) -> bool {
    value.is_null()
        || value.as_bool() == Some(false)
        || value.as_str() == Some("")
        || value.as_f64() == Some(0.0)
}

fn coordinate(value: Option<&Value>, field: &str) -> Result<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
        .ok_or_else(|| {
            invalid(format!(
                "{field} must be a finite normalized coordinate from 0 to 1"
            ))
        })
}
