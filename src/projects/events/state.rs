use super::*;
use crate::projects::api::ProjectsInner;
pub(super) fn project_by_id(inner: &ProjectsInner, data: &Value) -> Option<Project> {
    let id = data.get("jobID")?.as_str()?.to_uppercase();
    inner.projects.read().get(&id).cloned()
}

pub(super) fn handle_job_state(inner: &Arc<ProjectsInner>, data: &Value) {
    let Some(kind) = data.get("type").and_then(Value::as_str) else {
        return;
    };
    let Some(project) = project_by_id(inner, data) else {
        return;
    };
    match kind {
        "queued" => {
            let seconds =
                number(data.get("estimatedStartSeconds")).filter(|seconds| *seconds >= 0.0);
            let queue_status = data
                .get("queueStatus")
                .and_then(Value::as_str)
                .filter(|value| matches!(*value, "waiting" | "no-workers"))
                .map(ToOwned::to_owned);
            project.update(
                |state| {
                    if state.status.is_finished() {
                        return;
                    }
                    state.status = ProjectStatus::Queued;
                    state.queue_position = data
                        .get("queuePosition")
                        .and_then(Value::as_i64)
                        .unwrap_or(-1);
                    state.estimated_start_at = seconds.and_then(|seconds| {
                        chrono::Duration::milliseconds((seconds * 1_000.0) as i64)
                            .to_std()
                            .ok()
                            .and_then(|duration| chrono::Duration::from_std(duration).ok())
                            .map(|duration| Utc::now() + duration)
                    });
                    state.queue_status = queue_status;
                },
                &["status", "queuePosition", "estimatedStartAt", "queueStatus"],
            );
        }
        "jobCompleted" => {
            project.update(
                |state| {
                    if !state.status.is_finished() {
                        state.status = ProjectStatus::Completed;
                    }
                },
                &["status"],
            );
        }
        "initiatingModel" | "jobStarted" => {
            let Some(job_id) = data.get("imgID").and_then(Value::as_str) else {
                return;
            };
            let job = project.ensure_job(&job_id.to_uppercase());
            let status = if kind == "initiatingModel" {
                JobStatus::Initiating
            } else {
                JobStatus::Processing
            };
            job.update(
                |state| {
                    if state.status.is_finished() {
                        return;
                    }
                    state.status = status;
                    state.worker_name = data
                        .get("workerName")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                    for field in [
                        "positivePrompt",
                        "negativePrompt",
                        "jobIndex",
                        "preparation",
                    ] {
                        if let Some(value) = data.get(field) {
                            state.extra.insert(field.into(), value.clone());
                        }
                    }
                },
                &["status", "workerName", "preparation"],
            );
            project.update(
                |state| {
                    if state.status.is_finished() {
                        return;
                    }
                    state.status = ProjectStatus::Processing;
                    state.estimated_start_at = None;
                    state.queue_status = None;
                },
                &["status", "jobs"],
            );
        }
        _ => return,
    }
    inner.events.emit("project", data.clone());
}

pub(super) fn handle_job_progress(inner: &Arc<ProjectsInner>, data: &Value) {
    let Some(project) = project_by_id(inner, data) else {
        return;
    };
    let Some(job_id) = data.get("imgID").and_then(Value::as_str) else {
        return;
    };
    let job = project.ensure_job(&job_id.to_uppercase());
    job.update(
        |state| {
            if state.status.is_finished() {
                return;
            }
            state.status = JobStatus::Processing;
            if let Some(step) = number(data.get("step")) {
                state.step = state.step.max(step);
            }
            if let Some(step_count) = number(data.get("stepCount")) {
                state.step_count = step_count;
            }
            if let Some(progress) = number(data.get("progress")) {
                state.external_progress = Some(progress);
            }
            if let (Some(min), Some(max)) = (number(data.get("etaMin")), number(data.get("etaMax")))
            {
                state.eta_range = Some(json!({"min": min, "max": max}));
            }
        },
        &["status", "step", "stepCount", "progress"],
    );
    project.update(
        |state| {
            if !state.status.is_finished() {
                state.status = ProjectStatus::Processing;
            }
        },
        &["status", "jobs"],
    );
    inner.events.emit("job", data.clone());
    if data.get("hasImage").and_then(Value::as_bool) == Some(true) {
        let inner = inner.clone();
        tokio::spawn(async move {
            let query = json!({
                "jobId": project.id(),
                "imageId": job.id(),
                "type": "preview",
            });
            if let Ok(response) = inner
                .client
                .rest
                .get("/v1/image/downloadUrl", Some(&query))
                .await
            {
                if let Some(url) = response
                    .pointer("/data/downloadUrl")
                    .and_then(Value::as_str)
                {
                    job.update(
                        |state| state.preview_url = Some(url.to_owned()),
                        &["previewUrl"],
                    );
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

pub(super) fn handle_job_eta(inner: &Arc<ProjectsInner>, data: &Value) {
    let Some(project) = project_by_id(inner, data) else {
        return;
    };
    let Some(job_id) = data.get("imgID").and_then(Value::as_str) else {
        return;
    };
    let Some(seconds) = number(data.get("etaSeconds")) else {
        return;
    };
    let eta = Utc::now() + chrono::Duration::milliseconds((seconds * 1_000.0) as i64);
    let job = project.ensure_job(&job_id.to_uppercase());
    job.update(
        |state| {
            state.eta = Some(eta);
            state.eta_seconds = Some(seconds);
        },
        &["eta", "etaSeconds"],
    );
    let max_eta = project
        .jobs()
        .iter()
        .filter_map(|job| job.snapshot().eta)
        .max();
    project.update(|state| state.eta = max_eta, &["eta", "jobs"]);
    inner.events.emit("job", data.clone());
}
