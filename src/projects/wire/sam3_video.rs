use super::*;

pub(super) fn apply_selection(
    params: &Map<String, Value>,
    keyframe: &mut Map<String, Value>,
) -> Result<()> {
    let invalid = |message: &str| Error::InvalidInput(message.into());
    if !truthy(params.get("referenceVideo"))
        || params.get("numberOfMedia").and_then(Value::as_u64) != Some(1)
    {
        return Err(invalid(
            "Video selection requires one uploaded referenceVideo and numberOfMedia: 1.",
        ));
    }
    let selection = params
        .get("sam3VideoPrompt")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| invalid("Video selection requires a sam3VideoPrompt object."))?;
    let has_text_prompt = ["positivePrompt", "negativePrompt", "stylePrompt"]
        .iter()
        .any(|field| {
            params
                .get(*field)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        });
    if has_text_prompt
        || ["videoStart", "teacacheThreshold"]
            .iter()
            .any(|field| params.get(*field).is_some_and(|value| !value.is_null()))
        || [
            "referenceImage",
            "referenceImageEnd",
            "referenceAudio",
            "controlNet",
            "trimEndFrame",
            "referenceAudioIdentity",
            "referenceMask",
            "startingImage",
        ]
        .iter()
        .any(|field| truthy(params.get(*field)))
        || params
            .get("loras")
            .and_then(Value::as_array)
            .is_some_and(|values| !values.is_empty())
    {
        return Err(invalid(
            "Video selection accepts one complete source video and selection prompts.",
        ));
    }
    let opt_in = ["mask", "objects", "combine", "frames", "memory"]
        .iter()
        .any(|key| selection.contains_key(*key));
    let (prompt, frame_index) = if opt_in {
        // The service validates advanced selections against the actual source.
        (Value::Object(selection), None)
    } else {
        let (prompt, index) = normalize_single_selection(selection)?;
        (prompt, Some(index))
    };
    keyframe.extend([
        ("hasReferenceVideo".into(), json!(true)),
        ("sam3VideoPrompt".into(), prompt),
        ("steps".into(), json!(1)),
        ("seed".into(), json!(0)),
        ("interpolation".into(), json!("none")),
        ("generateAudio".into(), json!(false)),
    ]);
    for field in ["width", "height", "frames", "fps"] {
        if let Some(value) = params.get(field) {
            let valid = if field == "fps" {
                value
                    .as_f64()
                    .is_some_and(|fps| fps.is_finite() && fps > 0.0)
            } else {
                positive_or_zero_integer(value).is_some_and(|size| size > 0)
            };
            if !valid {
                return Err(Error::InvalidInput(format!(
                    "Video selection {field} must match the source video."
                )));
            }
            keyframe.insert(field.into(), value.clone());
        }
    }
    if frame_index
        .zip(params.get("frames").and_then(positive_or_zero_integer))
        .is_some_and(|(index, frames)| index >= frames)
    {
        return Err(invalid("frameIndex must refer to a source frame."));
    }
    Ok(())
}

fn normalize_single_selection(mut selection: Map<String, Value>) -> Result<(Value, u64)> {
    let invalid = |message: &str| Error::InvalidInput(message.into());
    let index = |value: Option<Value>, max: u64| {
        value.map_or(Some(0), |value| {
            positive_or_zero_integer(&value).filter(|index| *index <= max)
        })
    };
    let frame_index = index(selection.remove("frameIndex"), 899);
    let object_index = index(selection.remove("objectIndex"), 31);
    let (Some(frame_index), Some(object_index)) = (frame_index, object_index) else {
        return Err(invalid(
            "Use a valid source frameIndex and objectIndex from 0 to 31.",
        ));
    };
    if selection
        .remove("output")
        .is_some_and(|value| value != "mask")
        || selection
            .remove("edges")
            .is_some_and(|value| value != "hard")
    {
        return Err(invalid("Video selection supports hard mask output."));
    }
    if truthy(selection.get("applyMask"))
        || selection
            .get("maxInstances")
            .is_some_and(|value| positive_or_zero_integer(value) != Some(1))
    {
        return Err(invalid(
            "Video selection tracks one object; choose it with objectIndex.",
        ));
    }
    selection.remove("maxInstances");
    let mut prompt = normalize_sam3_prompt(&Value::Object(selection))?;
    if prompt.get("text").is_none() {
        let has_points = prompt["points"]
            .as_array()
            .is_some_and(|points| !points.is_empty());
        if has_points && object_index != 0 {
            return Err(invalid("Point selections require objectIndex 0."));
        }
        if !has_points && object_index >= prompt["boxes"].as_array().map_or(0, Vec::len) as u64 {
            return Err(invalid("objectIndex must refer to a supplied box."));
        }
    }
    prompt["maxInstances"] = json!(1);
    prompt["frameIndex"] = json!(frame_index);
    prompt["objectIndex"] = json!(object_index);
    prompt["output"] = json!("mask");
    prompt["edges"] = json!("hard");

    Ok((prompt, frame_index))
}

fn positive_or_zero_integer(value: &Value) -> Option<u64> {
    value
        .as_f64()
        .filter(|value| value.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(value))
        .map(|value| value as u64)
}
