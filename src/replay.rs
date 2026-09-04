use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{ApiError, Error, Result, transport::ApiClient, utils::path_segment};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ReplayWriteResult {
    pub run_id: String,
    pub schema_version: Value,
    pub redacted: bool,
    pub create_time: f64,
    pub update_time: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ReplayGetResult {
    pub record: Value,
    pub create_time: f64,
}

/// RunRecord ingestion and replay-viewer read APIs.
#[derive(Clone)]
pub struct ReplayApi {
    client: Arc<ApiClient>,
}

impl std::fmt::Debug for ReplayApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayApi").finish_non_exhaustive()
    }
}

impl ReplayApi {
    pub(crate) fn new(client: Arc<ApiClient>) -> Self {
        Self { client }
    }

    pub async fn write(&self, record: &Value) -> Result<ReplayWriteResult> {
        if !record.is_object() {
            return Err(Error::InvalidInput("record must be a JSON object".into()));
        }
        let response = self.client.rest.post("/v1/replay/records", record).await?;
        let run_id = response
            .get("runId")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("Replay write response missing runId"))?
            .to_owned();
        Ok(ReplayWriteResult {
            run_id,
            schema_version: response.get("schemaVersion").cloned().unwrap_or(json!(0)),
            redacted: response.get("redacted").and_then(Value::as_bool) == Some(true),
            create_time: finite_number(response.get("createTime")),
            update_time: finite_number(response.get("updateTime")),
        })
    }

    pub async fn list(&self, limit: Option<u32>) -> Result<Vec<Value>> {
        let query = limit
            .filter(|limit| *limit > 0)
            .map(|limit| json!({"limit": limit}));
        let response = self
            .client
            .rest
            .get("/v1/replay/records", query.as_ref())
            .await?;
        Ok(response
            .get("records")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub async fn get(&self, run_id: &str) -> Result<ReplayGetResult> {
        if run_id.trim().is_empty() {
            return Err(Error::InvalidInput("run_id is required".into()));
        }
        let response = self
            .client
            .rest
            .get(
                &format!("/v1/replay/records/{}", path_segment(run_id)),
                None,
            )
            .await?;
        let record = response
            .get("record")
            .filter(|record| record.is_object())
            .cloned()
            .ok_or_else(|| malformed("Replay get response missing record field"))?;
        Ok(ReplayGetResult {
            record,
            create_time: finite_number(response.get("createTime")),
        })
    }
}

fn finite_number(value: Option<&Value>) -> f64 {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
}

fn malformed(message: &str) -> Error {
    ApiError::new(
        500,
        json!({"status": "error", "message": message, "errorCode": 0}),
    )
    .into()
}
