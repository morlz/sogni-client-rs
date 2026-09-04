use std::{path::Path, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde_json::{Value, json};

use crate::{Error, Result};

const MAX_VISION_IMAGES: usize = 20;
const MAX_VISION_BYTES: usize = 10 * 1024 * 1024;

pub(super) async fn normalize_vision_messages(messages: &[Value]) -> Result<Vec<Value>> {
    let mut normalized = Vec::with_capacity(messages.len());
    let mut image_count = 0;
    for message in messages {
        let Some(parts) = message.get("content").and_then(Value::as_array) else {
            normalized.push(message.clone());
            continue;
        };
        let mut message = message.clone();
        let mut normalized_parts = Vec::with_capacity(parts.len());
        for part in parts {
            if part.get("type").and_then(Value::as_str) != Some("image_url") {
                normalized_parts.push(part.clone());
                continue;
            }
            image_count += 1;
            if image_count > MAX_VISION_IMAGES {
                return Err(Error::InvalidInput(format!(
                    "at most {MAX_VISION_IMAGES} vision images are allowed"
                )));
            }
            let source = part
                .pointer("/image_url/url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::InvalidInput("image_url.url is required".into()))?;
            let data_uri = inline_image(source).await?;
            let mut image_url = json!({"url": data_uri});
            if let Some(detail) = part.pointer("/image_url/detail") {
                image_url["detail"] = detail.clone();
            }
            normalized_parts.push(json!({"type": "image_url", "image_url": image_url}));
        }
        message["content"] = Value::Array(normalized_parts);
        normalized.push(message);
    }
    Ok(normalized)
}

async fn inline_image(source: &str) -> Result<String> {
    if source.starts_with("data:") {
        let mime = source
            .strip_prefix("data:")
            .and_then(|value| value.split_once(';'))
            .map(|(mime, _)| mime)
            .unwrap_or("");
        if !matches!(mime, "image/png" | "image/jpeg" | "image/jpg") {
            return Err(Error::InvalidInput(
                "vision chat supports PNG and JPEG images only".into(),
            ));
        }
        if let Some(encoded) = source.split_once(',').map(|(_, value)| value) {
            let approximate_bytes = encoded.len().saturating_mul(3) / 4;
            if approximate_bytes > MAX_VISION_BYTES {
                return Err(Error::InvalidInput("vision image exceeds 10 MB".into()));
            }
        }
        return Ok(source.to_owned());
    }
    let (data, declared_type) = if source.starts_with("http://") || source.starts_with("https://") {
        let response = reqwest::Client::new()
            .get(source)
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_VISION_BYTES as u64)
        {
            return Err(Error::InvalidInput("vision image exceeds 10 MB".into()));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(ToOwned::to_owned);
        (response.bytes().await?, content_type)
    } else {
        let path = Path::new(source);
        let data = Bytes::from(tokio::fs::read(path).await?);
        let content_type = mime_guess::from_path(path)
            .first_raw()
            .map(ToOwned::to_owned);
        (data, content_type)
    };
    if data.len() > MAX_VISION_BYTES {
        return Err(Error::InvalidInput("vision image exceeds 10 MB".into()));
    }
    let mime = if data.starts_with(b"\xFF\xD8\xFF")
        || matches!(declared_type.as_deref(), Some("image/jpg" | "image/jpeg"))
    {
        "image/jpeg"
    } else if data.starts_with(b"\x89PNG") || declared_type.as_deref() == Some("image/png") {
        "image/png"
    } else {
        return Err(Error::InvalidInput(
            "vision chat supports PNG and JPEG images only".into(),
        ));
    };
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(data)))
}

pub(super) fn redact_inline_images(messages: &mut [Value]) {
    for message in messages {
        let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts {
            if part.get("type").and_then(Value::as_str) == Some("image_url") {
                part["image_url"] = json!({"url": "[image]"});
            }
        }
    }
}
