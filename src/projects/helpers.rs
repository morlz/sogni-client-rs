use super::*;
pub(super) fn job_progress(state: &JobSnapshot) -> u8 {
    if state.status == JobStatus::Completed {
        return 100;
    }
    if let Some(progress) = state.external_progress.filter(|value| value.is_finite()) {
        let progress = if (0.0..=1.0).contains(&progress) {
            progress * 100.0
        } else {
            progress
        };
        return progress.round().clamp(0.0, 100.0) as u8;
    }
    if state.step_count > 0.0 {
        return (state.step / state.step_count * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8;
    }
    0
}

pub(super) fn expected_jobs(params: &Value) -> u32 {
    params
        .get("numberOfMedia")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(1)
}

pub(super) fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(truthy_value)
}

pub(super) fn truthy_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Number(value) => value.as_f64() != Some(0.0),
    }
}

pub(super) fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
        })
        .filter(|value| value.is_finite())
}

pub(super) fn copy_if_present(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    source_name: &str,
    target_name: &str,
) {
    if let Some(value) = source.get(source_name).filter(|value| !value.is_null()) {
        target.insert(target_name.into(), value.clone());
    }
}

pub(super) fn required_str<'a>(map: &'a Map<String, Value>, field: &str) -> Result<&'a str> {
    map.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() || field == "positivePrompt")
        .ok_or_else(|| Error::InvalidInput(format!("{field} is required")))
}

pub(super) fn required_str_value<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Error::InvalidInput(format!("{field} is required")))
}

pub(super) fn required_value<'a>(value: &'a Value, field: &str) -> Result<&'a Value> {
    value
        .get(field)
        .filter(|value| !value.is_null())
        .ok_or_else(|| Error::InvalidInput(format!("{field} is required")))
}

pub(super) fn required_number(value: &Value, field: &str) -> Result<f64> {
    number(value.get(field))
        .ok_or_else(|| Error::InvalidInput(format!("{field} must be a finite number")))
}

pub(super) fn require_nonempty(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(Error::InvalidInput(format!("{field} is required")))
    } else {
        Ok(())
    }
}
