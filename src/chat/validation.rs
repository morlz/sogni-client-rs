use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::{ChatError, Error, Result, WorkloadAttribution};

pub(super) fn assert_chat_run_external_media(params: &Value) -> Result<()> {
    let mut violations = Vec::new();
    for (message_index, message) in params
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (part_index, part) in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if part.get("type").and_then(Value::as_str) == Some("image_url") {
                validate_external_url(
                    part.pointer("/image_url/url"),
                    format!("messages[{message_index}].content[{part_index}].image_url.url"),
                    &mut violations,
                );
            }
        }
    }
    let references = alias(params, "mediaReferences", "media_references").and_then(Value::as_array);
    for (index, reference) in references.into_iter().flatten().enumerate() {
        validate_external_url(
            reference.get("url"),
            format!("mediaReferences[{index}].url"),
            &mut violations,
        );
        if reference.get("dataUri").is_some() || reference.get("data_uri").is_some() {
            violations.push(format!("mediaReferences[{index}].dataUri"));
        }
    }
    if let Some(context) = alias(params, "mediaContext", "media_context") {
        for field in [
            "images",
            "videos",
            "audio",
            "uploadedImages",
            "uploadedVideos",
            "uploadedAudio",
        ] {
            for (index, value) in context
                .get(field)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                validate_external_url(
                    Some(value),
                    format!("mediaContext.{field}[{index}]"),
                    &mut violations,
                );
            }
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "durable chat runs require HTTP(S) media URLs; invalid fields: {}",
            violations.join(", ")
        )))
    }
}

fn validate_external_url(value: Option<&Value>, path: String, violations: &mut Vec<String>) {
    if value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .is_some_and(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value.starts_with("http://") || value.starts_with("https://"))
        })
    {
        violations.push(path);
    }
}

pub(super) fn parse_attribution(value: Option<&Value>) -> Result<Option<WorkloadAttribution>> {
    value
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value(value.clone()).map_err(Error::from))
        .transpose()
}

pub(super) fn map_chat_error(result: Result<Value>) -> Result<Value> {
    match result {
        Err(Error::Api(error)) => {
            Err(ChatError::from_payload(error.payload, Some(error.status), None).into())
        }
        other => other,
    }
}

pub(super) fn run_field(response: &Value) -> Result<Value> {
    response
        .pointer("/data/run")
        .cloned()
        .ok_or_else(|| Error::Protocol("chat run response missing data.run".into()))
}

pub(super) fn alias<'a>(value: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    value.get(camel).or_else(|| value.get(snake))
}

pub(super) fn normalize_sogni_tools(value: Option<&Value>) -> Option<Value> {
    value.map(|value| {
        if value
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case("rich"))
        {
            json!("creative-tools")
        } else {
            value.clone()
        }
    })
}

pub(super) fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Error::InvalidInput(format!("{field} is required")))
}

pub(super) fn require_object(value: &Value, name: &str) -> Result<()> {
    if value.is_object() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!("{name} must be a JSON object")))
    }
}

pub(super) fn reject_untyped_tool_controls(params: &Value) -> Result<()> {
    require_object(params, "chat params")?;
    const CONTROLS: &[&str] = &[
        "autoExecuteTools",
        "auto_execute_tools",
        "onToolCall",
        "on_tool_call",
        "onToolProgress",
        "on_tool_progress",
        "maxToolRounds",
        "max_tool_rounds",
    ];
    let present = CONTROLS
        .iter()
        .filter(|field| params.get(**field).is_some())
        .copied()
        .collect::<Vec<_>>();
    if present.is_empty() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "{} are client-side controls and cannot be supplied as JSON; use create_completion_with_custom_tools() and ChatAutoToolOptions",
            present.join(", ")
        )))
    }
}

pub(super) fn require_nonempty(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(Error::InvalidInput(format!("{name} is required")))
    } else {
        Ok(())
    }
}

pub(super) fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) -> Result<()> {
    let name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|error| Error::InvalidInput(format!("invalid header name: {error}")))?;
    let value = HeaderValue::from_str(value)
        .map_err(|error| Error::InvalidInput(format!("invalid header value: {error}")))?;
    headers.insert(name, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_runs_reject_data_uris() {
        let error = assert_chat_run_external_media(&json!({
            "messages": [{"role": "user", "content": [{
                "type": "image_url", "image_url": {"url": "data:image/png;base64,AA=="}
            }]}]
        }))
        .expect_err("durable media must be retrievable");
        assert!(error.to_string().contains("HTTP(S)"));
    }

    #[test]
    fn untyped_tool_controls_fail_instead_of_being_dropped() {
        let error = reject_untyped_tool_controls(&json!({"autoExecuteTools": true}))
            .expect_err("raw callback controls must be rejected");
        assert!(
            error
                .to_string()
                .contains("create_completion_with_custom_tools")
        );
        assert!(reject_untyped_tool_controls(&json!({"model": "test"})).is_ok());
    }
}
