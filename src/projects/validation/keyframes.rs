use super::*;

pub(super) fn validate_keyframes(params: &Map<String, Value>, model_id: &str) -> Result<()> {
    let Some(value) = params.get("keyframes").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    if value.as_array().is_some_and(Vec::is_empty) {
        return Ok(());
    }
    if !is_minimax_h3_keyframe_model(model_id) {
        return Err(Error::InvalidInput(format!(
            "keyframes is supported only by the MiniMax H3 image-to-video, first/last-frame, Sound to Video and Reference to Video workflows (i2v, flf2v, ia2v, flfa2v, a2v and r2v model ids); {model_id} does not accept keyframes."
        )));
    }
    let entries = value.as_array().ok_or_else(|| {
        Error::InvalidInput("keyframes must be an array of { image, frameIndex } entries.".into())
    })?;
    if entries.len() > MINIMAX_H3_MAX_KEYFRAMES {
        return Err(Error::InvalidInput(format!(
            "keyframes accepts at most {MINIMAX_H3_MAX_KEYFRAMES} entries (got {}).",
            entries.len()
        )));
    }
    for (index, entry) in entries.iter().enumerate() {
        if !truthy(entry.get("image")) {
            return Err(Error::InvalidInput(format!(
                "keyframes[{index}].image is required."
            )));
        }
    }
    Ok(())
}

pub(in crate::projects) fn keyframe_indices(
    params: &Map<String, Value>,
    frames: Option<&Value>,
    duration: Option<f64>,
) -> Result<Vec<i64>> {
    let Some(entries) = params
        .get("keyframes")
        .and_then(Value::as_array)
        .filter(|entries| !entries.is_empty())
    else {
        return Ok(Vec::new());
    };
    let frames = frames.and_then(Value::as_i64).ok_or_else(|| {
        Error::InvalidInput("keyframes need the video length: pass frames or duration.".into())
    })?;
    let last_index = frames - 2;
    let video = duration.map_or_else(
        || format!("a {frames}-frame video"),
        |duration| format!("the {frames}-frame video that duration {duration} resolves to"),
    );
    let mut used = HashSet::new();
    let mut indices = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let value = entry.get("frameIndex").unwrap_or(&Value::Null);
        let number = value.as_f64();
        if !number.is_some_and(|number| {
            number.fract() == 0.0 && number >= 1.0 && number <= last_index as f64
        }) {
            let hint = if number == Some(0.0) || number == Some((frames - 1) as f64) {
                let model_id = required_str(params, "modelId")?;
                match get_video_workflow_type(model_id) {
                    Some("i2v" | "flf2v" | "flfa2v") => {
                        "; use referenceImage and referenceImageEnd for the first and last frames"
                            .into()
                    }
                    Some("ia2v") => format!(
                        "; use referenceImage for the first frame, and the last frame ({}) cannot be pinned",
                        frames - 1
                    ),
                    _ => format!("; frames 0 and {} cannot be pinned", frames - 1),
                }
            } else {
                String::new()
            };
            let got = match value {
                Value::Null => "nothing".into(),
                Value::Array(_) => "an array".into(),
                Value::Object(_) => "an object".into(),
                Value::Number(_) => number.expect("JSON number").to_string(),
                _ => value.to_string(),
            };
            return Err(Error::InvalidInput(format!(
                "keyframes[{index}].frameIndex must be an integer between 1 and {last_index} for {video} (got {got}){hint}."
            )));
        }
        let frame_index = number.expect("validated integer") as i64;
        if !used.insert(frame_index) {
            return Err(Error::InvalidInput(format!(
                "keyframes must use different frames; frame {frame_index} is used twice."
            )));
        }
        indices.push(frame_index);
    }
    Ok(indices)
}
