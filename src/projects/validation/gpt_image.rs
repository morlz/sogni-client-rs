use super::*;

pub(in crate::projects) fn validate_gpt_image_options(
    params: &Map<String, Value>,
    model_id: &str,
) -> Result<()> {
    let mask = truthy(params.get("gptImageMask"));
    let mask_url = truthy(params.get("gptImageMaskUrl"));
    let invalid = |message: &str| Error::InvalidInput(message.into());
    if !is_gpt_image_model(model_id) {
        return if mask || mask_url {
            Err(invalid("GPT Image masks require a GPT Image model"))
        } else {
            Ok(())
        };
    }
    if mask && mask_url {
        return Err(invalid("Provide one GPT Image mask"));
    }
    let context = params.get("contextImages").and_then(Value::as_array);
    let has_first =
        context.is_some_and(|images| !images.is_empty()) || truthy(params.get("contextImage1"));
    if mask && !has_first {
        return Err(invalid("GPT Image mask requires a first reference image"));
    }
    if params.contains_key("contextImages")
        && context.is_none_or(|images| {
            images.len() > 16 || images.iter().any(|value| !truthy_value(value))
        })
    {
        return Err(invalid(
            "GPT Image accepts up to 16 non-empty references in source order",
        ));
    }
    if let Some(url) = params.get("gptImageMaskUrl") {
        if !has_first || url.as_str().is_none_or(|url| url.trim().is_empty()) {
            return Err(invalid(
                "GPT Image mask requires a mask URL and a first reference image",
            ));
        }
    }
    let is_25 = model_id != "gpt-image-2";
    if let Some(quality) = params.get("gptImageQuality") {
        if quality.as_str() == Some("auto") {
            return Err(Error::InvalidInput(format!(
                "Unsupported quality for {model_id}: auto. Choose low, medium or high{}.",
                if is_25 { ", xhigh or max" } else { "" }
            )));
        }
        if !quality.as_str().is_some_and(|quality| {
            ["low", "medium", "high", "standard", "hd"].contains(&quality)
                || is_25 && ["xhigh", "max"].contains(&quality)
        }) {
            return Err(Error::InvalidInput(format!(
                "Unsupported quality for {model_id}: {}",
                scalar_string(quality)
            )));
        }
    }
    if let Some(background) = params.get("gptImageBackground") {
        if !background.as_str().is_some_and(|background| {
            ["opaque", "auto"].contains(&background) || is_25 && background == "transparent"
        }) {
            return Err(Error::InvalidInput(format!(
                "Unsupported background for {model_id}: {}",
                scalar_string(background)
            )));
        }
        if background == "transparent"
            && params.get("outputFormat").and_then(Value::as_str) == Some("jpg")
        {
            return Err(invalid("Transparent GPT Image output requires PNG or WebP"));
        }
    }
    if let Some(compression) = params.get("gptImageOutputCompression") {
        if compression.as_u64().is_none_or(|value| value > 100) {
            return Err(invalid(
                "GPT Image output compression must be an integer from 0 to 100",
            ));
        }
        if !matches!(
            params.get("outputFormat").and_then(Value::as_str),
            Some("jpg" | "webp")
        ) {
            return Err(invalid(
                "GPT Image output compression requires JPEG or WebP",
            ));
        }
    }
    Ok(())
}
