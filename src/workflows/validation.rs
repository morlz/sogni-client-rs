use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use crate::{ApiError, Error, Result};

pub(super) fn assert_external_media(references: &[Value]) -> Result<()> {
    let mut violations = Vec::new();
    for (index, reference) in references.iter().enumerate() {
        let Some(reference) = reference.as_object() else {
            continue;
        };
        if let Some(url) = reference.get("url").and_then(Value::as_str) {
            let url = url.trim().to_ascii_lowercase();
            if !(url.is_empty() || url.starts_with("http://") || url.starts_with("https://")) {
                violations.push(format!("media_references[{index}].url"));
            }
        }
        for key in ["dataUri", "data_uri"] {
            if reference
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
            {
                violations.push(format!("media_references[{index}].{key}"));
            }
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "durable workflows require HTTP(S) media URLs; invalid fields: {}",
            violations.join(", ")
        )))
    }
}

fn workflow_data(response: &Value) -> Result<&Map<String, Value>> {
    response
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Protocol("creative workflow response did not include data".into()))
}

pub(super) fn workflow_field(response: &Value, key: &str) -> Result<Value> {
    workflow_data(response)?
        .get(key)
        .cloned()
        .ok_or_else(|| Error::Protocol(format!("creative workflow response missing data.{key}")))
}

pub(super) fn template_data(response: &Value) -> &Value {
    if response.get("status").and_then(Value::as_str) == Some("success") {
        response
            .get("data")
            .filter(|value| value.is_object())
            .unwrap_or(response)
    } else {
        response
    }
}

pub(super) fn valid_template(value: &Value) -> bool {
    value.get("id").and_then(Value::as_str).is_some()
        && value.get("name").and_then(Value::as_str).is_some()
}

pub(super) fn required_template(response: &Value, operation: &str) -> Result<Value> {
    let template = template_data(response).get("template");
    if template.is_some_and(valid_template) {
        return Ok(template.cloned().expect("checked Some"));
    }
    let operation = if operation.is_empty() {
        String::new()
    } else {
        format!(" {operation}")
    };
    Err(ApiError::new(
        500,
        json!({
            "status": "error",
            "message": format!("Workflow template{operation} response missing template field"),
            "errorCode": 0,
        }),
    )
    .into())
}

pub(super) fn require_id(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(Error::InvalidInput(format!("{name} is required")))
    } else {
        Ok(())
    }
}

pub(super) fn require_object(value: &Value, name: &str) -> Result<()> {
    if value.is_object() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!("{name} must be a JSON object")))
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
    fn rejects_inline_media_for_durable_workflows() {
        let error = assert_external_media(&[json!({"url": "data:image/png;base64,abc"})])
            .expect_err("data URI must be rejected");
        assert!(error.to_string().contains("HTTP(S)"));
    }
}
