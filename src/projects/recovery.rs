use super::*;

/// Stable marker stored in `originalCode` when reconciliation proves a project was lost.
pub const PROJECT_LOST_ORIGINAL_CODE: &str = "projectLost";

/// Event emitted with newly rehydrated in-flight recovery records.
pub const ACTIVE_PROJECTS_RECOVERED_EVENT: &str = "activeProjectsRecovered";

/// Event emitted with newly rehydrated completed recovery records.
pub const COMPLETED_PROJECTS_RECOVERED_EVENT: &str = "completedProjectsRecovered";

/// Retry policy for [`ProjectsApi::resolve_missing`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolveMissingOptions {
    /// Number of REST lookups before consulting the socket's active-project list.
    pub attempts: usize,
    /// Delay between REST lookups after the first attempt.
    pub retry_delay: Duration,
}

impl Default for ResolveMissingOptions {
    fn default() -> Self {
        Self {
            attempts: MISSING_PROJECT_ATTEMPTS,
            retry_delay: MISSING_PROJECT_RETRY,
        }
    }
}

/// Outcome of looking up a project omitted from a recovery snapshot.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ProjectResolution {
    /// The durable REST record confirms a terminal project (success, failure or cancellation).
    Finished { project: Value },
    /// The v2 REST status or socket registry still owns the in-flight project.
    Active,
    /// Neither the durable REST API nor the socket currently has a record.
    /// This does not establish that a historical project never existed.
    Lost,
    /// A non-404 error prevented a reliable verdict.
    Unknown { error: String },
}

impl ProjectResolution {
    /// Stable lower-case state name matching the JavaScript and Python clients.
    #[must_use]
    pub const fn state(&self) -> &'static str {
        match self {
            Self::Finished { .. } => "finished",
            Self::Active => "active",
            Self::Lost => "lost",
            Self::Unknown { .. } => "unknown",
        }
    }
}

pub(super) fn project_lost_payload() -> Value {
    json!({
        "code": 0,
        "originalCode": PROJECT_LOST_ORIGINAL_CODE,
        "message": "The server has no record of this generation. It may have been interrupted by a restart — please try again."
    })
}

/// Returns `true` for the failure assigned when reconciliation cannot find a project.
#[must_use]
pub fn is_project_lost_error(error: &Error) -> bool {
    matches!(
        error,
        Error::Project(error)
            if error.payload.get("originalCode").and_then(Value::as_str)
                == Some(PROJECT_LOST_ORIGINAL_CODE)
    )
}

/// Returns `true` when a raw project-error payload carries the lost-project marker.
#[must_use]
pub fn is_project_lost_payload(payload: &Value) -> bool {
    payload.get("originalCode").and_then(Value::as_str) == Some(PROJECT_LOST_ORIGINAL_CODE)
}

pub(super) fn recovered_params(raw: &Value) -> Value {
    let request = raw
        .get("clientRequestData")
        .and_then(Value::as_str)
        .and_then(|value| crate::utils::b64_json_decode(value).ok())
        .unwrap_or_else(|| json!({}));
    let keyframe = request
        .pointer("/keyFrames/0")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let model = raw.get("model").cloned().unwrap_or_else(|| json!({}));
    let model_type = raw
        .get("modelType")
        .or_else(|| model.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("image");
    let media_type = match model_type {
        "video" => "video",
        "audio" | "music" => "audio",
        _ => "image",
    };
    let count = raw
        .get("imageCount")
        .and_then(Value::as_u64)
        .or_else(|| request.get("numberOfImages").and_then(Value::as_u64))
        .unwrap_or(1);
    let mut params = json!({
        "type": media_type,
        "modelId": keyframe.get("modelID").or_else(|| model.get("id")).cloned().unwrap_or(json!("")),
        "positivePrompt": keyframe.get("positivePrompt").cloned().unwrap_or(json!("")),
        "numberOfMedia": count,
    });
    for (source, target) in [
        ("negativePrompt", "negativePrompt"),
        ("stylePrompt", "stylePrompt"),
        ("guidanceScale", "guidance"),
        ("seed", "seed"),
        ("loras", "loras"),
        ("loraStrengths", "loraStrengths"),
    ] {
        if let Some(value) = keyframe.get(source).filter(|value| !value.is_null()) {
            params[target] = value.clone();
        }
    }
    if let Some(steps) = raw.get("stepCount").or_else(|| keyframe.get("steps")) {
        params["steps"] = steps.clone();
    }
    for field in ["network", "tokenType", "billingMode", "appSource"] {
        if let Some(value) = raw.get(field).or_else(|| request.get(field)) {
            params[field] = value.clone();
        }
    }
    if let Some(value) = request.get("outputFormat") {
        params["outputFormat"] = value.clone();
    }
    params
}

pub(super) fn replay_recovered(project: &Project, raw: &Value, completed: bool) {
    let mut jobs = Vec::new();
    if !completed {
        jobs.extend(
            raw.get("workerJobs")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
    }
    jobs.extend(
        raw.get("completedWorkerJobs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    );
    for job_raw in jobs {
        let Some(id) = job_raw
            .get("imgID")
            .or_else(|| job_raw.get("id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let job = project.ensure_job(&id.to_uppercase());
        let raw_status = job_raw.get("status").and_then(Value::as_str).unwrap_or("");
        let status = match raw_status {
            "assigned" | "initiatingModel" => JobStatus::Initiating,
            "jobStarted" | "jobProgress" => JobStatus::Processing,
            "jobCompleted" => JobStatus::Completed,
            "jobError" => JobStatus::Failed,
            _ => JobStatus::Pending,
        };
        job.update(
            |state| {
                if state.status.is_finished() && !status.is_finished() {
                    return;
                }
                if !state.status.is_finished() {
                    state.status = status;
                }
                state.step = number(job_raw.get("performedSteps")).unwrap_or(state.step);
                state.seed = job_raw
                    .get("seedUsed")
                    .and_then(Value::as_i64)
                    .or(state.seed);
                state.worker_name = job_raw
                    .pointer("/worker/username")
                    .or_else(|| job_raw.pointer("/worker/name"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or_else(|| state.worker_name.clone());
                state.result_url = raw_result_url(&job_raw).or_else(|| state.result_url.clone());
                state.is_nsfw = job_raw
                    .get("triggeredNSFWFilter")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                state.nsfw_detected = job_raw
                    .get("nsfwDetected")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                state.nsfw_sources = job_raw
                    .get("nsfwSources")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect();
            },
            &["status", "step", "resultUrl"],
        );
    }
    let status = match raw.get("status").and_then(Value::as_str) {
        Some("completed") => Some(ProjectStatus::Completed),
        Some("errored" | "failed") => Some(ProjectStatus::Failed),
        Some("cancelled" | "canceled") => Some(ProjectStatus::Canceled),
        Some("queued" | "active") => Some(ProjectStatus::Queued),
        Some("processing") => Some(ProjectStatus::Processing),
        Some("pending") => Some(ProjectStatus::Pending),
        _ if completed => Some(ProjectStatus::Completed),
        _ => None,
    };
    if let Some(status) = status {
        project.update(
            |state| {
                if !state.status.is_finished() {
                    state.status = status;
                }
            },
            &["status", "jobs"],
        );
    }
}

pub(super) fn is_llm_recovery(raw: &Value) -> bool {
    raw.get("jobType").and_then(Value::as_str) == Some("llm")
        || raw.pointer("/model/type").and_then(Value::as_str) == Some("llm")
}

pub(super) fn raw_result_url(data: &Value) -> Option<String> {
    [
        "resultUrl",
        "imageUrl",
        "imageFile",
        "videoUrl",
        "videoFile",
    ]
    .iter()
    .find_map(|field| {
        data.get(field)
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(ToOwned::to_owned)
    })
}
