use super::*;
impl ProjectsApi {
    /// Merge ready private imports into the public catalog for this call only.
    /// Public-only callers can keep using `available_loras` unchanged.
    pub async fn available_loras_with_personal(&self, model_id: Option<&str>) -> Result<Value> {
        let session = self.inner.client.rest.auth_updates();
        let mut catalog = self.available_loras(model_id).await?;
        if !catalog.is_object() {
            return Err(Error::Protocol(
                "public LoRA catalog must be an object".into(),
            ));
        }
        let personal = self.personal_loras().catalog(None).await?;
        personal_loras::check_session(&session)?;
        let mut models = catalog
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        let mut loras = catalog
            .get("loras")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for row in personal["loras"].as_array().expect("validated catalog") {
            models.extend(
                row.get("modelIds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
            if model_id.is_none_or(|model| compatible(row, model)) {
                loras.push(row.clone());
            }
        }
        catalog["loras"] = json!(loras);
        catalog["models"] = json!(models);
        Ok(catalog)
    }

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
        let catalog = if lora_id.starts_with("personal-") {
            self.personal_loras().catalog(None).await?
        } else {
            self.available_loras(None).await?
        };
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

pub(super) fn compatible(row: &Value, model_id: &str) -> bool {
    row.get("modelIds")
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(model_id)))
}
