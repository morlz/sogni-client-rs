use super::state::project_by_id;
use super::*;
use crate::projects::api::ProjectsInner;
pub(super) async fn handle_job_result(inner: &Arc<ProjectsInner>, data: &Value) {
    let Some(project) = project_by_id(inner, data) else {
        return;
    };
    let Some(job_id) = data.get("imgID").and_then(Value::as_str) else {
        return;
    };
    let job = project.ensure_job(&job_id.to_uppercase());
    let nsfw = data
        .get("triggeredNSFWFilter")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let detected = data.get("nsfwDetected").and_then(Value::as_bool) == Some(true);
    let canceled = data.get("userCanceled").and_then(Value::as_bool) == Some(true);
    let mut result_url = raw_result_url(data);
    job.update(
        |state| {
            state.status = if canceled {
                JobStatus::Canceled
            } else {
                JobStatus::Completed
            };
            state.result_url.clone_from(&result_url);
            state.is_nsfw = nsfw;
            state.nsfw_detected = detected;
        },
        &["status", "resultUrl", "isNSFW", "nsfwDetected"],
    );
    if result_url.is_none() && (!nsfw || detected) && !canceled {
        if let Ok(url) = job.get_result_url().await {
            result_url = Some(url);
        }
    }
    let seed = data
        .get("lastSeed")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()));
    let steps = number(data.get("performedStepCount"));
    job.update(
        |state| {
            if let Some(seed) = seed {
                state.seed = Some(seed);
            }
            if let Some(steps) = steps {
                state.step = steps;
            }
            state.result_url = result_url;
            state.is_nsfw = nsfw;
            state.nsfw_detected = detected;
            state.nsfw_sources = data
                .get("nsfwSources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect();
        },
        &["status", "resultUrl", "isNSFW", "nsfwDetected"],
    );
    project.notify("jobCompleted", json!({"jobId": job_id}));
    inner.events.emit("job", data.clone());
}

pub(super) fn handle_job_error(inner: &Arc<ProjectsInner>, data: &Value) {
    let Some(project) = project_by_id(inner, data) else {
        return;
    };
    let raw_code = data.get("error").cloned().unwrap_or(json!(5000));
    let code = raw_code
        .as_i64()
        .or_else(|| raw_code.as_str().and_then(|code| code.parse().ok()));
    let symbolic = match raw_code.as_str() {
        Some("serverRestarting") => 5001,
        Some("workerDisconnected") => 5002,
        Some("jobTimedOut") => 5003,
        Some("artistCanceled") => 5004,
        Some("workerCancelled") => 5005,
        _ => 5000,
    };
    let mut error = json!({
        "code": code.unwrap_or(symbolic),
        "message": data.get("error_message").and_then(Value::as_str).unwrap_or("Project failed"),
    });
    if code.is_none() {
        error["originalCode"] = raw_code;
    }
    for key in [
        "subscriptionLimit",
        "requiredPlans",
        "feature",
        "limitation",
    ] {
        if let Some(value) = data.get(key) {
            error[key] = value.clone();
        }
    }
    if let Some(job_id) = data.get("imgID").and_then(Value::as_str) {
        let job = project.ensure_job(&job_id.to_uppercase());
        job.update(
            |state| {
                state.status = JobStatus::Failed;
                state.error = Some(error.clone());
            },
            &["status", "error"],
        );
        let snapshot = project.snapshot();
        let all_started = snapshot.jobs.len() >= expected_jobs(&snapshot.params) as usize;
        if expected_jobs(&snapshot.params) == 1
            || all_started
                && snapshot
                    .jobs
                    .iter()
                    .all(|job| job.status == JobStatus::Failed)
        {
            project.update(
                |state| {
                    state.status = ProjectStatus::Failed;
                    state.error = Some(error.clone());
                },
                &["status", "error"],
            );
        }
        project.notify("jobFailed", json!({"jobId": job_id, "error": error}));
    } else {
        project.update(
            |state| {
                state.status = ProjectStatus::Failed;
                state.error = Some(error.clone());
            },
            &["status", "error"],
        );
    }
    inner.events.emit("job", data.clone());
}

pub(in crate::projects) async fn cancel_project(
    inner: &Arc<ProjectsInner>,
    project_id: &str,
) -> Result<()> {
    if inner
        .projects
        .read()
        .get(project_id)
        .is_some_and(|project| project.status().is_finished())
    {
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + CANCEL_TIMEOUT;
    let mut events = inner.client.subscribe();
    inner
        .client
        .send_socket(
            "jobError",
            &json!({
                "jobID": project_id,
                "error": "artistCanceled",
                "error_message": "artistCanceled",
                "isFromWorker": false,
            }),
        )
        .await?;
    tokio::time::timeout_at(deadline, async {
        loop {
            let event = events.recv().await.map_err(|error| {
                Error::Transport(format!("cancellation event stream closed: {error}"))
            })?;
            if event.name != "artistCancelConfirmation"
                || event.data.get("jobID").and_then(Value::as_str) != Some(project_id)
            {
                continue;
            }
            if event.data.get("didCancel").and_then(Value::as_bool) == Some(true) {
                return Ok(());
            }
            return Err(Error::Protocol(
                event
                    .data
                    .get("error_message")
                    .and_then(Value::as_str)
                    .unwrap_or("project cancellation was not confirmed")
                    .to_owned(),
            ));
        }
    })
    .await
    .map_err(|_| Error::Timeout("project cancellation was not confirmed".into()))??;
    let api = ProjectsApi {
        inner: inner.clone(),
    };
    let terminal = tokio::time::timeout_at(deadline, terminal_after_cancel(&api, project_id))
        .await
        .map_err(|_| Error::Timeout("project cancellation has not finished".into()))??;
    if let Some(project) = inner.projects.read().get(project_id).cloned() {
        replay_recovered(&project, &terminal, false);
        let status = match project.status() {
            ProjectStatus::Canceled => JobStatus::Canceled,
            ProjectStatus::Failed => JobStatus::Failed,
            _ => return Ok(()),
        };
        for job in project.jobs() {
            if !job.status().is_finished() {
                job.update(|state| state.status = status, &["status"]);
            }
        }
    }
    Ok(())
}

async fn terminal_after_cancel(api: &ProjectsApi, project_id: &str) -> Result<Value> {
    loop {
        let status = api.get_status(project_id).await?;
        if status.get("finished").and_then(Value::as_bool) == Some(true) {
            return Ok(status);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
#[path = "cancel_tests.rs"]
mod cancel_tests;
