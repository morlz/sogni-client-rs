//! Private LoRA imports. Eligibility, validation and limits remain server-owned.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Error, Result, transport::RestClient, utils::path_segment};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonalLora {
    pub id: String,
    pub name: String,
    pub model_id: String,
    pub model_ids: Vec<String>,
    pub source: String,
    /// queued, validating, review, ready, rejected, or revoked.
    pub status: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Coarse, user-actionable failure category supplied by the service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    pub requirements: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PersonalLoraLimits {
    pub entries: u64,
    pub file_bytes: Option<u64>,
    pub imports_per_day: u64,
    pub per_generation: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PersonalLoraLibrary {
    pub loras: Vec<PersonalLora>,
    /// Discover supported import targets here instead of hard-coding model IDs.
    pub models: Vec<String>,
    pub limits: PersonalLoraLimits,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportPersonalLoraParams {
    /// Public Hugging Face safetensors or Civitai model/version link.
    pub url: String,
    pub name: String,
    pub model_id: String,
    /// Explicit permission to use the file on Sogni; never inferred by the SDK.
    pub rights_confirmed: bool,
}

/// Manage private imports using the current API key or account session.
/// Responses are never cached or returned across an account change.
#[derive(Clone, Debug)]
pub struct PersonalLoras {
    rest: RestClient,
}

impl PersonalLoras {
    pub(super) fn new(rest: RestClient) -> Self {
        Self { rest }
    }

    /// List imports and statuses, including after a subscription lapses.
    pub async fn list(&self) -> Result<PersonalLoraLibrary> {
        Ok(serde_json::from_value(
            self.read("/v1/loras/personal").await?,
        )?)
    }

    pub async fn get(&self, id: &str) -> Result<PersonalLora> {
        super::require_nonempty(id, "id")?;
        Ok(serde_json::from_value(
            self.read(&format!("/v1/loras/personal/{}", path_segment(id)))
                .await?,
        )?)
    }

    /// Start an import, then poll `get` until ready, rejected, or revoked.
    pub async fn import(&self, params: &ImportPersonalLoraParams) -> Result<PersonalLora> {
        let session = self.rest.auth_updates();
        let response = self
            .rest
            .post("/v1/loras/personal", &serde_json::to_value(params)?)
            .await?;
        check_session(&session)?;
        Ok(serde_json::from_value(data(response)?)?)
    }

    pub async fn remove(&self, id: &str) -> Result<()> {
        super::require_nonempty(id, "id")?;
        self.rest
            .delete(&format!("/v1/loras/personal/{}", path_segment(id)))
            .await?;
        Ok(())
    }

    /// Ready imports with their current model compatibility and strength ranges.
    pub async fn catalog(&self, model_id: Option<&str>) -> Result<Value> {
        let response = self.read("/v1/loras/personal/catalog").await?;
        let loras = response
            .get("loras")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Protocol("personal LoRA catalog missing loras".into()))?;
        Ok(json!({"loras": loras.iter().filter(|row| {
            model_id.is_none_or(|model| super::loras::compatible(row, model))
        }).cloned().collect::<Vec<_>>()}))
    }

    async fn read(&self, path: &str) -> Result<Value> {
        let session = self.rest.auth_updates();
        let response = self.rest.get(path, None).await?;
        check_session(&session)?;
        data(response)
    }
}

pub(super) fn check_session(session: &tokio::sync::watch::Receiver<u64>) -> Result<()> {
    if session.has_changed().unwrap_or(true) {
        return Err(Error::InvalidInput(
            "The account changed. Refresh your LoRA library.".into(),
        ));
    }
    Ok(())
}

fn data(response: Value) -> Result<Value> {
    response
        .get("data")
        .filter(|data| data.is_object())
        .cloned()
        .ok_or_else(|| Error::Protocol("personal LoRA response missing data".into()))
}
