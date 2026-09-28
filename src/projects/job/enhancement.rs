use super::*;

impl Job {
    pub(super) async fn enhancement_size(
        &self,
        params: &Value,
        api: &ProjectsApi,
    ) -> Result<Option<(Value, Value)>> {
        let preset = params
            .get("sizePreset")
            .and_then(Value::as_str)
            .filter(|preset| !preset.is_empty() && *preset != "custom");
        let Some(preset) = preset else {
            return Ok(params
                .get("width")
                .filter(|value| truthy_value(value))
                .zip(params.get("height").filter(|value| truthy_value(value)))
                .map(|(width, height)| (width.clone(), height.clone())));
        };
        let model = required_str_value(params, "modelId")?;
        let network = if params.get("network").and_then(Value::as_str) == Some("relaxed") {
            Network::Relaxed
        } else {
            Network::Fast
        };
        let presets = api.get_size_presets(network, model).await?;
        let selected = presets
            .iter()
            .find(|value| value.get("id").and_then(Value::as_str) == Some(preset))
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "Size preset \"{preset}\" is not available for {model}"
                ))
            })?;
        Ok(Some((
            required_value(selected, "width")?.clone(),
            required_value(selected, "height")?.clone(),
        )))
    }
}
