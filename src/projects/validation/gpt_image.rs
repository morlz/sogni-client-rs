use super::*;

pub(in crate::projects) fn gif_dimensions(
    params: &Map<String, Value>,
) -> Result<Option<(u32, u32)>> {
    if params.get("outputFormat").and_then(Value::as_str) != Some("gif")
        || (params
            .get("sizePreset")
            .and_then(Value::as_str)
            .is_some_and(|preset| !preset.is_empty() && preset != "custom")
            && !params.contains_key("width")
            && !params.contains_key("height"))
    {
        return Ok(None);
    }
    let width = optional_integer(params, "width", 1, u32::MAX)?;
    let height = optional_integer(params, "height", 1, u32::MAX)?;
    let count = optional_integer(params, "gptImageFrameCount", 2, 16)?;
    resolve_gpt_image_gif_size(width, height, count, None).map(Some)
}

fn optional_integer(
    params: &Map<String, Value>,
    field: &str,
    min: u32,
    max: u32,
) -> Result<Option<u32>> {
    params
        .get(field)
        .map(|value| {
            value
                .as_f64()
                .filter(|value| {
                    value.fract() == 0.0 && (f64::from(min)..=f64::from(max)).contains(value)
                })
                .map(|value| value as u32)
                .ok_or_else(|| {
                    Error::InvalidInput(format!("{field} must be an integer from {min} to {max}"))
                })
        })
        .transpose()
}

fn validate_advanced_options(
    params: &Map<String, Value>,
    model_id: &str,
    mask: bool,
    has_first: bool,
) -> Result<()> {
    let gif = params.get("outputFormat").and_then(Value::as_str) == Some("gif");
    let complexity = params.get("gptImageComplexity");
    let markup = params.get("gptImageMarkupUrl");
    let frames =
        params.contains_key("gptImageFrameCount") || params.contains_key("gptImageFrameDurationMs");
    if !(gif || frames || complexity.is_some() || markup.is_some()) {
        return Ok(());
    }
    let invalid = |message: &str| Error::InvalidInput(message.into());
    if !crate::utils::is_gpt_image_26_model(model_id) {
        return Err(invalid(
            "GIF, complexity and markup options require GPT Image 2.6",
        ));
    }
    if complexity.is_some_and(|value| !matches!(value.as_str(), Some("low" | "medium" | "high"))) {
        return Err(invalid("Unsupported GPT Image complexity"));
    }
    if gif && (mask || markup.is_some()) {
        return Err(invalid("GIF output cannot combine with masks or markup"));
    }
    if !gif && frames {
        return Err(invalid("Frame options require GIF output"));
    }
    optional_integer(params, "gptImageFrameCount", 2, 16)?;
    optional_integer(params, "gptImageFrameDurationMs", 20, 2000)?;
    if markup.is_some_and(|value| {
        value.as_str().is_none_or(|url| url.trim().is_empty()) || !has_first || mask
    }) {
        return Err(invalid(
            "Markup requires a first edit reference and cannot combine with a mask",
        ));
    }
    Ok(())
}

pub(in crate::projects) fn validate_gpt_image_options(
    params: &Map<String, Value>,
    model_id: &str,
) -> Result<()> {
    let mask = truthy(params.get("gptImageMask"));
    let mask_url = truthy(params.get("gptImageMaskUrl"));
    let invalid = |message: &str| Error::InvalidInput(message.into());
    let context = params.get("contextImages").and_then(Value::as_array);
    let has_first =
        context.is_some_and(|images| !images.is_empty()) || truthy(params.get("contextImage1"));
    validate_advanced_options(params, model_id, mask || mask_url, has_first)?;
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
            && matches!(
                params.get("outputFormat").and_then(Value::as_str),
                Some("jpg" | "gif")
            )
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
