use super::*;

pub(in crate::projects) fn validate_seedream_options(
    params: &Map<String, Value>,
    model_id: &str,
) -> Result<()> {
    if !crate::utils::is_seedream_model(model_id) {
        return Ok(());
    }
    if params.get("contextImages").is_some_and(|value| {
        value.as_array().is_none_or(|images| {
            images.len() > 10 || images.iter().any(|image| !truthy_value(image))
        })
    }) {
        return Err(Error::InvalidInput(format!(
            "{model_id} accepts up to 10 non-empty reference images in source order"
        )));
    }
    let custom = params.get("sizePreset").is_none()
        || params.get("sizePreset").and_then(Value::as_str) == Some("custom");
    if !custom || !truthy(params.get("width")) || !truthy(params.get("height")) {
        return Ok(());
    }
    let width = number(params.get("width")).unwrap_or(f64::NAN);
    let height = number(params.get("height")).unwrap_or(f64::NAN);
    let pixels = width * height;
    if !(921_600.0..=4_194_304.0).contains(&pixels) || width.max(height) / width.min(height) > 16.0
    {
        return Err(Error::InvalidInput(format!(
            "{model_id} renders 921,600 to 4,194,304 pixels at an aspect ratio from 1:16 to 16:1; got {width}x{height}"
        )));
    }
    Ok(())
}
