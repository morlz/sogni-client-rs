use super::*;
impl ProjectsApi {
    pub async fn get_supported_models(&self, force_refresh: bool) -> Result<Vec<Value>> {
        if !force_refresh {
            if let Some(cached) = fresh_cache(&self.inner.supported_models) {
                return Ok(value_array(&cached));
            }
        }
        let value = self
            .inner
            .client
            .socket_get("/api/v1/models/list", None)
            .await?;
        *self.inner.supported_models.write() = Some(TimedValue {
            value: value.clone(),
            loaded_at: Instant::now(),
        });
        Ok(value_array(&value))
    }

    async fn get_model_tiers(&self, force_refresh: bool) -> Result<Value> {
        if !force_refresh {
            if let Some(cached) = fresh_cache(&self.inner.model_tiers) {
                return Ok(cached);
            }
        }
        let value = self
            .inner
            .client
            .socket_get("/api/v2/models/tiers", None)
            .await?;
        *self.inner.model_tiers.write() = Some(TimedValue {
            value: value.clone(),
            loaded_at: Instant::now(),
        });
        Ok(value)
    }

    pub async fn get_model_options(
        &self,
        model_id: &str,
        force_refresh: bool,
    ) -> Result<ModelOptions> {
        require_nonempty(model_id, "model_id")?;
        let (models, tiers) = tokio::try_join!(
            self.get_supported_models(force_refresh),
            self.get_model_tiers(force_refresh)
        )?;
        let model = models
            .iter()
            .find(|model| model.get("id").and_then(Value::as_str) == Some(model_id))
            .ok_or_else(|| Error::InvalidInput(format!("model {model_id} is not supported")))?;
        let tier_id = model
            .get("tier")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol(format!("model {model_id} has no tier")))?;
        let tier = tiers
            .get(tier_id)
            .ok_or_else(|| Error::Protocol(format!("model tier {tier_id} was not returned")))?;
        let media_type = tier
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("image")
            .to_owned();
        let raw = map_model_options(tier, &media_type);
        Ok(ModelOptions {
            model_id: model_id.to_owned(),
            media_type,
            raw,
        })
    }

    pub async fn get_size_presets(&self, network: Network, model_id: &str) -> Result<Vec<Value>> {
        let value = self
            .inner
            .client
            .socket_get(
                &format!(
                    "/api/v1/size-presets/network/{}/model/{}",
                    network.as_str(),
                    path_segment(model_id)
                ),
                None,
            )
            .await?;
        Ok(value_array(&value))
    }

    pub async fn get_available_models(&self, network: Network) -> Result<Vec<Value>> {
        let workers = self
            .inner
            .client
            .socket_get(
                &format!("/api/v1/status/network/{}/models", network.as_str()),
                None,
            )
            .await?;
        let models = self.get_supported_models(false).await?;
        let mut result = Vec::new();
        if let Some(workers) = workers.as_object() {
            for (sid, count) in workers {
                let model = models.iter().find(|model| {
                    model.get("SID").and_then(Value::as_i64) == sid.parse::<i64>().ok()
                });
                result.push(json!({
                    "id": model.and_then(|m| m.get("id")).cloned().unwrap_or_else(|| json!(sid)),
                    "name": model.and_then(|m| m.get("name")).cloned().unwrap_or_else(|| json!(sid.replace('-', " "))),
                    "workerCount": count,
                    "media": model.and_then(|m| m.get("media")).cloned().unwrap_or_else(|| json!("image")),
                }));
            }
        }
        Ok(result)
    }

    pub async fn get_video_asset_config(&self, model_id: &str) -> Result<Value> {
        if !self.is_video_model_id(model_id) {
            return Err(Error::InvalidInput(format!(
                "model {model_id} is not a video model"
            )));
        }
        let workflow = get_video_workflow_type(model_id);
        Ok(json!({
            "workflowType": workflow,
            "assets": workflow.map(|workflow| video_asset_requirements(model_id, workflow)),
        }))
    }
}
