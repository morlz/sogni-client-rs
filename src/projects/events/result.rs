use super::state::project_by_id;
use super::*;
use crate::projects::api::ProjectsInner;
pub(super) async fn handle_job_result(inner: &Arc<ProjectsInner>, data: &Value) {
    let session = inner.client.rest.request_session();
    let Some(job_id) = data.get("imgID").and_then(Value::as_str) else {
        return;
    };
    let Some(project) = project_by_id(inner, data) else {
        let Some(project_id) = data.get("jobID").and_then(Value::as_str) else {
            return;
        };
        let mut event = data.clone();
        let mut url = raw_result_url(data);
        if url.is_none()
            && !(data.get("triggeredNSFWFilter").and_then(Value::as_bool) == Some(true)
                && data.get("nsfwDetected").and_then(Value::as_bool) != Some(true))
            && data.get("userCanceled").and_then(Value::as_bool) != Some(true)
        {
            if let Some(evidence) = api::result_media_evidence(data) {
                let api = ProjectsApi {
                    inner: inner.clone(),
                };
                let content_type = (evidence.kind == ResultMediaKind::Audio)
                    .then_some(evidence.content_type.as_deref())
                    .flatten();
                url = session
                    .run(api.mint_result_url(project_id, job_id, evidence.kind, content_type))
                    .await
                    .ok();
            }
        }
        if session.check().is_ok() {
            event["resultUrl"] = json!(url);
            inner.events.emit("job", event);
        }
        return;
    };
    let Some(job) = project.job_for_attempt(job_id, data.get("jobIndex").and_then(Value::as_u64))
    else {
        return;
    };
    project.clear_job_queue(job_id, data.get("jobIndex").and_then(Value::as_u64));
    job.record_result_evidence(data);
    let nsfw = data
        .get("triggeredNSFWFilter")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let detected = data.get("nsfwDetected").and_then(Value::as_bool) == Some(true);
    let canceled = data.get("userCanceled").and_then(Value::as_bool) == Some(true);
    let provenance = JobProvenance::from_result(data);
    let result_url = raw_result_url(data);
    let seed = data
        .get("lastSeed")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()));
    let steps = number(data.get("performedStepCount"));
    // Completion may be observed while URL signing is still in flight. Publish
    // all synchronous result metadata together before the first await.
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
            state.nsfw_sources = data
                .get("nsfwSources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect();
            if let Some(seed) = seed {
                state.seed = Some(seed);
            }
            if let Some(steps) = steps {
                state.step = steps;
            }
            if provenance.is_some() {
                state.provenance.clone_from(&provenance);
            }
            copy_export_metadata(state, data);
        },
        &[
            "status",
            "resultUrl",
            "isNSFW",
            "nsfwDetected",
            "nsfwSources",
            "seed",
            "step",
            "provenance",
        ],
    );
    if result_url.is_none() && (!nsfw || detected) && !canceled {
        // get_result_url stores a successful URL itself. A failed lookup must
        // not erase a URL another waiter obtained while this signer was pending.
        let _ = job.get_result_url().await;
    }
    if session.check().is_err() || project.check_session().is_err() {
        return;
    }
    let mut completed = json!({"jobId": job_id});
    let mut event = data.clone();
    let snapshot = job.snapshot();
    event["resultUrl"] = json!(snapshot.result_url);
    for target in [&mut completed, &mut event] {
        if let Some(url) = &snapshot.last_frame_url {
            target["lastFrameUrl"] = json!(url);
        }
        if let Some(format) = &snapshot.output_format {
            target["outputFormat"] = json!(format);
        }
    }
    if let Some(provenance) = provenance {
        completed["provenance"] = json!(provenance);
        event["provenance"] = json!(provenance);
    }
    project.notify("jobCompleted", completed);
    inner.events.emit("job", event);
}

pub(in crate::projects) fn copy_export_metadata(state: &mut JobSnapshot, data: &Value) {
    let string = |field| {
        data.get(field)
            .or_else(|| data.get("result")?.get(field))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    state.last_frame_url = string("lastFrameUrl").or_else(|| state.last_frame_url.clone());
    state.last_frame_key = string("lastFrameKey").or_else(|| state.last_frame_key.clone());
    state.output_format = string("outputFormat").or_else(|| state.output_format.clone());
}

pub(super) fn handle_job_error(inner: &Arc<ProjectsInner>, data: &Value) {
    let api = ProjectsApi {
        inner: inner.clone(),
    };
    if api.resubmit_if_restarting(data) {
        return;
    }
    let Some(project) = project_by_id(inner, data) else {
        return;
    };
    inner.submission.lock().unadmitted.remove(&project.id());
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
        project.clear_job_queue(job_id, data.get("jobIndex").and_then(Value::as_u64));
        let Some(job) =
            project.job_for_attempt(job_id, data.get("jobIndex").and_then(Value::as_u64))
        else {
            return;
        };
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

#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;
