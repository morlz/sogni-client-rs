use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Pending,
    Queued,
    Processing,
    Completed,
    Failed,
    Canceled,
}

impl ProjectStatus {
    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Initiating,
    Processing,
    Completed,
    Failed,
    Canceled,
}

impl JobStatus {
    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    pub id: String,
    pub project_id: String,
    pub status: JobStatus,
    pub step: f64,
    pub step_count: f64,
    pub external_progress: Option<f64>,
    pub seed: Option<i64>,
    pub result_url: Option<String>,
    pub preview_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_frame_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_frame_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
    /// Public worker receipt, when this result includes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<JobProvenance>,
    pub error: Option<Value>,
    pub is_nsfw: bool,
    pub nsfw_detected: bool,
    pub nsfw_sources: Vec<String>,
    pub worker_name: Option<String>,
    pub eta: Option<DateTime<Utc>>,
    pub eta_seconds: Option<f64>,
    pub eta_range: Option<Value>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl JobSnapshot {
    /// Typed view of known preparation phases; raw future phases stay in `extra`.
    #[must_use]
    pub fn preparation(&self) -> Option<JobPreparation> {
        serde_json::from_value(self.extra.get("preparation")?.clone()).ok()
    }

    pub(super) fn pending(id: String, project_id: String, step_count: f64) -> Self {
        Self {
            id,
            project_id,
            status: JobStatus::Pending,
            step: 0.0,
            step_count,
            external_progress: None,
            seed: None,
            result_url: None,
            preview_url: None,
            last_frame_url: None,
            last_frame_key: None,
            output_format: None,
            provenance: None,
            error: None,
            is_nsfw: false,
            nsfw_detected: false,
            nsfw_sources: Vec::new(),
            worker_name: None,
            eta: None,
            eta_seconds: None,
            eta_range: None,
            extra: Map::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSnapshot {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub recovered: bool,
    pub params: Value,
    #[serde(rename = "type")]
    pub media_type: String,
    pub status: ProjectStatus,
    pub error: Option<Value>,
    pub eta: Option<DateTime<Utc>>,
    pub queue_position: i64,
    pub estimated_start_at: Option<DateTime<Utc>>,
    pub queue_status: Option<String>,
    pub jobs: Vec<JobSnapshot>,
    pub progress: u8,
    pub result_urls: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CostEstimate {
    pub token: Value,
    pub usd: Value,
    pub spark: Value,
    pub sogni: Value,
    pub estimated_render_seconds: Option<f64>,
    pub estimated_total_seconds: Option<f64>,
    pub raw: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ModelOptions {
    pub model_id: String,
    pub media_type: String,
    pub raw: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PresignedPost {
    pub url: Url,
    pub fields: BTreeMap<String, String>,
    pub max_size_bytes: Option<u64>,
    pub public_url: Option<Url>,
}
