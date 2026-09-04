use super::*;
impl ProjectsApi {
    /// Low-level estimator for forward-compatible socket quote endpoints.
    pub async fn estimate(&self, path: &str, query: Option<&Value>) -> Result<CostEstimate> {
        let response = self.inner.client.socket_get(path, query).await?;
        parse_cost(response)
    }

    pub async fn estimate_cost(&self, params: &Value) -> Result<CostEstimate> {
        let model = required_str_value(params, "model")?;
        self.get_model_options(model, false).await?;
        let network = params
            .get("network")
            .and_then(Value::as_str)
            .unwrap_or("fast");
        if !matches!(network, "fast" | "relaxed") {
            return Err(Error::InvalidInput(
                "network must be fast or relaxed".into(),
            ));
        }
        let token_type = params
            .get("tokenType")
            .and_then(Value::as_str)
            .unwrap_or("spark");
        let mut segments = vec![
            token_type.into(),
            network.into(),
            model.into(),
            scalar_string(required_value(params, "imageCount")?),
            scalar_string(required_value(params, "stepCount")?),
            scalar_string(required_value(params, "previewCount")?),
            if params.get("cnEnabled").and_then(Value::as_bool) == Some(true) {
                "1".into()
            } else {
                "0".into()
            },
            number(params.get("startingImageStrength"))
                .map(|value| 1.0 - value)
                .unwrap_or(0.0)
                .to_string(),
        ];
        if let Some(size_preset) = params.get("sizePreset").and_then(Value::as_str) {
            let network = if network == "relaxed" {
                Network::Relaxed
            } else {
                Network::Fast
            };
            let preset = self
                .get_size_presets(network, model)
                .await?
                .into_iter()
                .find(|preset| preset.get("id").and_then(Value::as_str) == Some(size_preset))
                .ok_or_else(|| Error::InvalidInput("invalid sizePreset".into()))?;
            segments.push(scalar_string(required_value(&preset, "width")?));
            segments.push(scalar_string(required_value(&preset, "height")?));
        } else {
            segments.push(
                params
                    .get("width")
                    .map(scalar_string)
                    .unwrap_or_else(|| "0".into()),
            );
            segments.push(
                params
                    .get("height")
                    .map(scalar_string)
                    .unwrap_or_else(|| "0".into()),
            );
        }
        let version = if params.get("sampler").is_some() || params.get("contextImages").is_some() {
            segments.extend([
                params
                    .get("guidance")
                    .map(scalar_string)
                    .unwrap_or_else(|| "0".into()),
                params
                    .get("sampler")
                    .map(scalar_string)
                    .unwrap_or_else(|| "_".into()),
                params
                    .get("contextImages")
                    .map(|value| {
                        value
                            .as_array()
                            .map_or_else(|| scalar_string(value), |items| items.len().to_string())
                    })
                    .unwrap_or_else(|| "0".into()),
            ]);
            3
        } else {
            2
        };
        let path = segments
            .iter()
            .map(|segment| path_segment(segment))
            .collect::<Vec<_>>()
            .join("/");
        let query = json!({
            "gptImageQuality": params.get("gptImageQuality"),
            "outputFormat": params.get("outputFormat"),
        });
        self.estimate(
            &format!("/api/v{version}/job/estimate/{path}"),
            Some(&query),
        )
        .await
    }

    pub async fn estimate_enhancement_cost(
        &self,
        strength: &str,
        token_type: &str,
    ) -> Result<CostEstimate> {
        let strength = enhancement_strength(strength);
        self.estimate_cost(&json!({
            "network": "fast",
            "tokenType": token_type,
            "model": "flux1-schnell-fp8",
            "imageCount": 1,
            "stepCount": 5,
            "previewCount": 0,
            "cnEnabled": false,
            "startingImageStrength": strength,
        }))
        .await
    }

    pub async fn estimate_video_cost(&self, params: &Value) -> Result<CostEstimate> {
        let token_type = required_value(params, "tokenType")?;
        let model = required_str_value(params, "model")?;
        let width = required_value(params, "width")?;
        let height = required_value(params, "height")?;
        let fps = required_number(params, "fps")?;
        let frames = params.get("frames").and_then(Value::as_i64).map_or_else(
            || calculate_video_frames(model, required_number(params, "duration")?, fps, None, None),
            Ok,
        )?;
        let mut segments = vec![
            scalar_string(token_type),
            model.to_owned(),
            scalar_string(width),
            scalar_string(height),
            frames.to_string(),
            fps.to_string(),
        ];
        let count = params
            .get("numberOfMedia")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        if let Some(steps) = params.get("steps") {
            segments.extend([scalar_string(steps), count.to_string()]);
        } else if count != 1 {
            segments.extend(["0".into(), count.to_string()]);
        }
        let encoded = segments
            .iter()
            .map(|segment| path_segment(segment))
            .collect::<Vec<_>>()
            .join("/");
        let query = json!({
            "hasVideoInput": params.get("hasVideoInput").and_then(Value::as_bool).filter(|v| *v).map(|_| 1),
            "referenceImageCount": params.get("referenceImageCount"),
            "referenceVideoCount": params.get("referenceVideoCount"),
            "referenceVideoDurationSeconds": params.get("referenceVideoDurationSeconds"),
        });
        self.estimate(
            &format!("/api/v1/job-video/estimate/{encoded}"),
            Some(&query),
        )
        .await
    }

    pub async fn estimate_audio_cost(&self, params: &Value) -> Result<CostEstimate> {
        let fields = ["tokenType", "model", "duration", "steps", "numberOfMedia"];
        let path = fields
            .iter()
            .map(|field| required_value(params, field).map(scalar_string))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .map(|segment| path_segment(segment))
            .collect::<Vec<_>>()
            .join("/");
        self.estimate(&format!("/api/v1/job-audio/estimate/{path}"), None)
            .await
    }
}
