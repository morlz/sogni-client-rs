use super::*;
use crate::projects::api::ProjectsInner;
mod result;
mod state;
pub(super) use result::cancel_project;
pub(super) use result::copy_export_metadata;
use result::{handle_job_error, handle_job_result};
use state::{handle_job_eta, handle_job_progress, handle_job_state};
pub(super) fn listen_for_project_events(inner: &Arc<ProjectsInner>) -> tokio::task::JoinHandle<()> {
    let mut receiver = inner.client.subscribe_scoped();
    let mut session = inner.client.rest.request_session();
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        loop {
            let incoming = tokio::select! {
                biased;
                () = session.changed() => {
                    let Some(inner) = weak.upgrade() else { return; };
                    let api = ProjectsApi { inner };
                    api.clear_previous_sessions();
                    session = api.inner.client.rest.request_session();
                    if session.is_closed() { return; }
                    continue;
                },
                event = receiver.recv() => event,
            };
            let scoped = match incoming {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        "project event receiver lagged; requesting recovery"
                    );
                    if let Some(inner) = weak.upgrade() {
                        let api = ProjectsApi { inner };
                        let owner = api.inner.client.rest.request_session();
                        if let Err(error) = api.sync("event-lagged").await {
                            if !ownerless_error(&error, &owner) {
                                tracing::warn!("project recovery after event lag failed");
                            }
                        }
                    }
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            let Some(inner) = weak.upgrade() else {
                return;
            };
            ProjectsApi {
                inner: inner.clone(),
            }
            .clear_previous_sessions();
            if scoped
                .session
                .is_some_and(|owner| owner != inner.client.auth_session())
            {
                continue;
            }
            let event = scoped.event;
            if matches!(
                event.name.as_str(),
                "jobState" | "jobProgress" | "jobETA" | "jobResult" | "jobRetry" | "projectQueue"
            ) || (event.name == "jobError"
                && !(event.data.get("imgID").and_then(Value::as_str).is_none()
                    && event
                        .data
                        .get("error")
                        .is_some_and(|code| code == 1001 || code == "1001")))
            {
                if let Some(id) = event.data.get("jobID").and_then(Value::as_str) {
                    inner.submission.lock().observed(&id.to_uppercase());
                }
            }
            match event.name.as_str() {
                "jobState" => handle_job_state(&inner, &event.data),
                "jobProgress" => handle_job_progress(&inner, &event.data),
                "jobETA" => handle_job_eta(&inner, &event.data),
                "jobResult" => {
                    let session = inner.client.rest.request_session();
                    let mut completion = Box::pin(async move {
                        let _ = session
                            .run(async {
                                handle_job_result(&inner, &event.data).await;
                                Ok(())
                            })
                            .await;
                    });
                    // Apply the result's synchronous state changes in socket order,
                    // then let URL signing wait independently of live queue updates.
                    if futures_util::poll!(completion.as_mut()).is_pending() {
                        tokio::spawn(completion);
                    }
                }
                "projectQueue" => {
                    if let Some(project) = state::project_by_id(&inner, &event.data) {
                        project.receive_queue(&event.data);
                    }
                }
                "disconnected" | "serverDisconnected" => {
                    for project in inner.projects.read().values() {
                        project.invalidate_queue();
                    }
                }
                "jobError" => handle_job_error(&inner, &event.data),
                "jobRetry" => handle_job_retry(&inner, &event.data),
                "changeNetwork" => {
                    if let Some(network) = event.data.get("network").and_then(Value::as_str) {
                        match network {
                            "fast" => *inner.runtime_network.write() = Some(Network::Fast),
                            "relaxed" => *inner.runtime_network.write() = Some(Network::Relaxed),
                            _ => {}
                        }
                    }
                    *inner.available_models.write() = Vec::new();
                    inner.events.emit("availableModels", json!([]));
                }
                "swarmModels" => handle_swarm_models(&inner, &event.data),
                "authenticated" => {
                    let api = ProjectsApi {
                        inner: inner.clone(),
                    };
                    tokio::spawn(async move {
                        let owner = api.inner.client.rest.request_session();
                        if let Err(error) = api.sync("authenticated").await {
                            if !ownerless_error(&error, &owner) {
                                tracing::warn!("project recovery failed");
                            }
                        }
                    });
                }
                _ => {}
            }
        }
    })
}

fn handle_job_retry(inner: &ProjectsInner, data: &Value) {
    // Retry diagnostics describe the abandoned attempt. Keep them internal so
    // public job consumers do not fail the render that is still being retried.
    if let Some(project) = state::project_by_id(inner, data) {
        project.retry_job(data);
    }
}

#[cfg(test)]
mod queue_tests;
#[cfg(test)]
mod result_api_tests;
#[cfg(test)]
mod retry_tests;
#[cfg(test)]
mod session_tests;

fn handle_swarm_models(inner: &Arc<ProjectsInner>, data: &Value) {
    let Some(workers) = data.as_object() else {
        return;
    };
    let workers = workers.clone();
    let inner = inner.clone();
    tokio::spawn(async move {
        let api = ProjectsApi {
            inner: inner.clone(),
        };
        let owner = inner.client.rest.request_session();
        let supported = match api.get_supported_models(false).await {
            Ok(models) => models,
            Err(error) => {
                if !ownerless_error(&error, &owner) {
                    tracing::warn!("failed to resolve live model metadata");
                }
                return;
            }
        };
        let models = workers
            .iter()
            .map(|(id, count)| {
                let model = supported
                    .iter()
                    .find(|model| model.get("id").and_then(Value::as_str) == Some(id));
                json!({
                    "id": id,
                    "name": model.and_then(|model| model.get("name")).cloned().unwrap_or_else(|| json!(id.replace('-', " "))),
                    "workerCount": count,
                    "media": model.and_then(|model| model.get("media")).cloned().unwrap_or_else(|| json!("image")),
                })
            })
            .collect::<Vec<_>>();
        *inner.available_models.write() = models.clone();
        inner.events.emit("availableModels", Value::Array(models));
    });
}

fn ownerless_error(error: &Error, owner: &crate::auth::RequestSession) -> bool {
    matches!(error, Error::Closed)
        || (matches!(error, Error::InvalidInput(_)) && owner.check().is_err())
}
