use super::*;
fn set_numbered_asset_param(
    params: &mut Map<String, Value>,
    first: &str,
    additional: &str,
    slot: u8,
) {
    match slot {
        1 => {
            params.insert(first.into(), json!(true));
        }
        2.. => {
            let values = params
                .entry(additional)
                .or_insert_with(|| Value::Array(Vec::new()));
            if !values.is_array() {
                *values = Value::Array(Vec::new());
            }
            let values = values.as_array_mut().expect("array inserted above");
            values.resize(usize::from(slot - 1), Value::Null);
            values[usize::from(slot - 2)] = json!(true);
        }
        _ => {}
    }
}

pub(in crate::projects) fn mark_asset_param(params: &mut Map<String, Value>, role: &AssetRole) {
    match role {
        AssetRole::ControlNetImage => {
            if let Some(control) = params.get_mut("controlNet").and_then(Value::as_object_mut) {
                control.insert("image".into(), json!(true));
            }
        }
        AssetRole::ReferenceAudioSlot(slot) => {
            set_numbered_asset_param(params, "referenceAudio", "referenceAudios", *slot);
        }
        AssetRole::ReferenceVideoSlot(slot) => {
            set_numbered_asset_param(params, "referenceVideo", "referenceVideos", *slot);
        }
        _ => {
            if let Some(name) = role.param_name() {
                params.insert(name, json!(true));
            }
        }
    }
}

pub(in crate::projects) fn effective_asset_wire_name(role: &AssetRole, model_id: &str) -> String {
    if is_minimax_h3_reference_model(model_id) {
        match role {
            AssetRole::ReferenceAudio => return "referenceAudio1".into(),
            AssetRole::ReferenceVideo => return "referenceVideo1".into(),
            _ => {}
        }
    }
    role.wire_name()
}

pub(in crate::projects) fn validate_asset_roles(
    model_id: &str,
    assets: &[(AssetRole, MediaSource)],
) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    let mut audio_slots = [false; 3];
    let mut video_slots = [false; 3];
    for (role, _) in assets {
        match role {
            AssetRole::ContextImage(slot) if !(1..=16).contains(slot) => {
                return Err(Error::InvalidInput(
                    "context image slot must be between 1 and 16".into(),
                ));
            }
            AssetRole::ReferenceAudioSlot(slot) | AssetRole::ReferenceVideoSlot(slot)
                if !(1..=3).contains(slot) =>
            {
                return Err(Error::InvalidInput(
                    "MiniMax H3 reference-media slot must be between 1 and 3".into(),
                ));
            }
            AssetRole::ReferenceAudioSlot(_) | AssetRole::ReferenceVideoSlot(_)
                if !is_minimax_h3_reference_model(model_id) =>
            {
                return Err(Error::InvalidInput(
                    "numbered reference-media slots are supported only by MiniMax H3 r2v models"
                        .into(),
                ));
            }
            _ => {}
        }
        let wire_name = effective_asset_wire_name(role, model_id);
        if !seen.insert(wire_name.clone()) {
            return Err(Error::InvalidInput(format!(
                "duplicate project asset role {wire_name}"
            )));
        }
        match role {
            AssetRole::ReferenceAudio if is_minimax_h3_reference_model(model_id) => {
                audio_slots[0] = true;
            }
            AssetRole::ReferenceAudioSlot(slot) => audio_slots[usize::from(*slot - 1)] = true,
            AssetRole::ReferenceVideo if is_minimax_h3_reference_model(model_id) => {
                video_slots[0] = true;
            }
            AssetRole::ReferenceVideoSlot(slot) => video_slots[usize::from(*slot - 1)] = true,
            _ => {}
        }
    }
    for (slots, label) in [(audio_slots, "audio"), (video_slots, "video")] {
        if let Some(last) = slots.iter().rposition(|present| *present) {
            if slots[..=last].iter().any(|present| !present) {
                return Err(Error::InvalidInput(format!(
                    "MiniMax H3 {label}-reference slots must be contiguous starting at 1"
                )));
            }
        }
    }
    Ok(())
}

impl ProjectRequest {
    #[must_use]
    pub fn image(model_id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self::new("image", model_id, prompt)
    }

    #[must_use]
    pub fn video(model_id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self::new("video", model_id, prompt)
    }

    #[must_use]
    pub fn audio(model_id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self::new("audio", model_id, prompt)
    }

    #[must_use]
    pub fn new(
        media_type: impl Into<String>,
        model_id: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        let mut params = Map::new();
        params.insert("type".into(), json!(media_type.into()));
        params.insert("modelId".into(), json!(model_id.into()));
        params.insert("positivePrompt".into(), json!(prompt.into()));
        params.insert("numberOfMedia".into(), json!(1));
        Self {
            params,
            assets: Vec::new(),
            attribution: None,
        }
    }

    pub fn from_value(value: Value) -> Result<Self> {
        let params = value
            .as_object()
            .cloned()
            .ok_or_else(|| Error::InvalidInput("project request must be a JSON object".into()))?;
        Ok(Self {
            params,
            assets: Vec::new(),
            attribution: None,
        })
    }

    #[must_use]
    pub fn param(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.params.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn number_of_media(self, value: u32) -> Self {
        self.param("numberOfMedia", value)
    }

    /// Select the source-image foreground for a SAM3 segmentation request.
    #[must_use]
    pub fn sam3_prompt(self, value: Sam3ImagePrompt) -> Self {
        self.param("sam3Prompt", json!(value))
    }

    /// Request worker-attested hashes for a Sogni World generation stage.
    #[must_use]
    pub fn world_generation_receipt(self, value: WorldGenerationReceiptRequest) -> Self {
        self.param("worldGenerationReceipt", json!(value))
    }

    #[must_use]
    pub fn steps(self, value: u32) -> Self {
        self.param("steps", value)
    }

    #[must_use]
    pub fn guidance(self, value: f64) -> Self {
        self.param("guidance", value)
    }

    #[must_use]
    pub fn dimensions(self, width: u32, height: u32) -> Self {
        self.param("width", width).param("height", height)
    }

    #[must_use]
    pub fn duration(self, seconds: f64) -> Self {
        self.param("duration", seconds)
    }

    #[must_use]
    pub fn fps(self, value: f64) -> Self {
        self.param("fps", value)
    }

    #[must_use]
    pub fn network(self, value: Network) -> Self {
        self.param("network", value.as_str())
    }

    #[must_use]
    pub fn asset(mut self, role: AssetRole, source: MediaSource) -> Self {
        mark_asset_param(&mut self.params, &role);
        self.assets.push((role, source));
        self
    }

    #[must_use]
    pub fn attribution(mut self, value: WorkloadAttribution) -> Self {
        self.attribution = Some(value);
        self
    }

    #[must_use]
    pub fn params(&self) -> Value {
        Value::Object(self.params.clone())
    }
}
