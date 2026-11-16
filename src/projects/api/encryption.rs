use super::*;
use crate::{MediaEncryptionApi, MediaEncryptionError as E, media_encryption::ProjectEncryption};

impl ProjectsApi {
    /// Read the model's advertised encryption capability. Only the Fast
    /// network currently supports encrypted projects.
    pub async fn supports_media_encryption(&self, model_id: &str) -> Result<bool> {
        Ok(self.encryption_inputs(model_id).await?.is_some())
    }

    async fn encryption_inputs(&self, model_id: &str) -> Result<Option<bool>> {
        let models = self.get_supported_models(false).await?;
        Ok(models
            .iter()
            .find(|model| {
                model.get("id").and_then(Value::as_str) == Some(model_id)
                    && model
                        .get("supportsMediaEncryption")
                        .and_then(Value::as_bool)
                        == Some(true)
            })
            .map(|model| {
                model.get("mediaEncryptionInputs").and_then(Value::as_bool) != Some(false)
            }))
    }

    pub(super) async fn project_encryption(
        &self,
        request: &ProjectRequest,
    ) -> Result<Option<ProjectEncryption>> {
        let mode = match request.params.get("mediaEncryption") {
            None | Some(Value::Null) => None,
            Some(Value::Bool(mode)) => Some(*mode),
            _ => {
                return Err(Error::InvalidInput(
                    "mediaEncryption must be a boolean".into(),
                ));
            }
        };
        let api = MediaEncryptionApi::new(self.inner.client.clone());
        if mode == Some(false) || (mode.is_none() && !api.automatic()) {
            return Ok(None);
        }
        let required = mode == Some(true);
        let unsupported = |reason| {
            if required {
                Err(E::Unsupported(reason).into())
            } else {
                Ok(None)
            }
        };
        let network = request.params.get("network").and_then(Value::as_str);
        let current = self
            .inner
            .runtime_network
            .read()
            .unwrap_or(self.inner.client.network());
        if network.unwrap_or(current.as_str()) != Network::Fast.as_str() {
            return unsupported("only the Fast network supports encrypted media");
        }
        if !required && !api.can_encrypt_automatically().await {
            return Ok(None);
        }
        let model_id = required_str(&request.params, "modelId")?;
        let Some(encrypt_inputs) = self.encryption_inputs(model_id).await? else {
            return unsupported("the selected model does not support media encryption");
        };
        if encrypt_inputs && has_preuploaded_input(request) {
            return unsupported("pass media bytes or files in place of pre-uploaded media");
        }
        api.start_project(encrypt_inputs).await.map(Some)
    }
}

// After request preparation, a true marker can represent either an attached
// Rust asset or externally uploaded bytes. Only the former can be encrypted.
fn has_preuploaded_input(request: &ProjectRequest) -> bool {
    let attached = |role: &AssetRole| {
        request
            .assets
            .iter()
            .any(|(candidate, _)| candidate == role)
    };
    let params = &request.params;
    let flag = |name: &str| params.get(name).and_then(Value::as_bool) == Some(true);
    let ordinary = [
        ("startingImage", AssetRole::StartingImage),
        ("gptImageMask", AssetRole::GptImageMask),
        ("leftViewImage", AssetRole::Pixal3dLeftView),
        ("backViewImage", AssetRole::Pixal3dBackView),
        ("rightViewImage", AssetRole::Pixal3dRightView),
        ("referenceImage", AssetRole::ReferenceImage),
        ("referenceImageEnd", AssetRole::ReferenceImageEnd),
        ("referenceMask", AssetRole::ReferenceMask),
        ("referenceAudio", AssetRole::ReferenceAudio),
        ("referenceAudioIdentity", AssetRole::ReferenceAudioIdentity),
        ("referenceVideo", AssetRole::ReferenceVideo),
    ];
    if ordinary
        .iter()
        .any(|(field, role)| flag(field) && !attached(role))
        || (params
            .get("controlNet")
            .and_then(|value| value.pointer("/image"))
            .and_then(Value::as_bool)
            == Some(true)
            && !attached(&AssetRole::ControlNetImage))
    {
        return true;
    }
    let missing = |field, to_role: fn(u8) -> AssetRole| {
        params
            .get(field)
            .and_then(Value::as_array)
            .is_some_and(|values| {
                values.iter().enumerate().any(|(index, value)| {
                    let media = value.get("image").unwrap_or(value);
                    media.as_bool() == Some(true)
                        && !u8::try_from(index + 1).is_ok_and(|slot| attached(&to_role(slot)))
                })
            })
    };
    missing("contextImages", AssetRole::ContextImage)
        || missing("referenceAudios", AssetRole::ReferenceAudioSlot)
        || missing("referenceVideos", AssetRole::ReferenceVideoSlot)
        || missing("keyframes", AssetRole::KeyframeImage)
        || (1..=16).any(|slot| {
            flag(format!("contextImage{slot}").as_str())
                && !attached(&AssetRole::ContextImage(slot))
                && !matches!(slot, 1 if attached(&AssetRole::Pixal3dLeftView))
                && !matches!(slot, 2 if attached(&AssetRole::Pixal3dBackView))
                && !matches!(slot, 3 if attached(&AssetRole::Pixal3dRightView))
        })
}
