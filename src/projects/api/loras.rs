use super::*;
impl ProjectsApi {
    pub async fn available_loras(&self, model_id: Option<&str>) -> Result<Value> {
        let query = model_id.map(|model_id| json!({"modelId": model_id}));
        let response = self
            .inner
            .client
            .rest
            .get("/v1/loras/comfy", query.as_ref())
            .await?;
        let data = response.get("data").cloned().unwrap_or_else(|| json!({}));
        if let Some(model_id) = model_id {
            let mut data = data.as_object().cloned().unwrap_or_default();
            let filtered = data
                .get("loras")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|item| {
                    item.get("modelIds")
                        .and_then(Value::as_array)
                        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(model_id)))
                })
                .cloned()
                .collect::<Vec<_>>();
            data.insert("loras".into(), Value::Array(filtered));
            return Ok(Value::Object(data));
        }
        Ok(data)
    }

    pub async fn get_lora(&self, lora_id: &str) -> Result<Option<Value>> {
        require_nonempty(lora_id, "lora_id")?;
        let catalog = self.available_loras(None).await?;
        Ok(catalog
            .get("loras")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|lora| lora.get("loraId").and_then(Value::as_str) == Some(lora_id))
            .cloned())
    }

    pub async fn supports_loras(&self, model_id: &str) -> Result<bool> {
        let catalog = self.available_loras(None).await?;
        Ok(catalog
            .get("models")
            .and_then(Value::as_array)
            .is_some_and(|models| models.iter().any(|model| model.as_str() == Some(model_id))))
    }

    pub async fn lora_constraints(&self) -> Result<Value> {
        let catalog = self.available_loras(None).await?;
        Ok(catalog.get("constraints").cloned().unwrap_or_else(
            || json!({"maxPerRequest": 8, "minStrength": -100, "maxStrength": 100}),
        ))
    }
}
