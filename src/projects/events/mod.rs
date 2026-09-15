use super::*;
use crate::projects::api::ProjectsInner;
mod result;
mod state;
pub(super) use result::cancel_project;
pub(super) use result::copy_export_metadata;
use result::{handle_job_error, handle_job_result};
use state::{handle_job_eta, handle_job_progress, handle_job_state};
pub(super) fn listen_for_project_events(inner: &Arc<ProjectsInner>) {
    let mut receiver = inner.client.subscribe();
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        loop {
            let event = match receiver.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        "project event receiver lagged; requesting recovery"
                    );
                    if let Some(inner) = weak.upgrade() {
                        let api = ProjectsApi { inner };
                        if api.sync("event-lagged").await.is_err() {
                            tracing::warn!("project recovery after event lag failed");
                        }
                    }
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if matches!(
                event.name.as_str(),
                "jobState" | "jobProgress" | "jobETA" | "jobResult" | "jobRetry"
            ) {
                if let Some(id) = event.data.get("jobID").and_then(Value::as_str) {
                    inner
                        .submission
                        .lock()
                        .unadmitted
                        .remove(&id.to_uppercase());
                }
            }
            match event.name.as_str() {
                "jobState" => handle_job_state(&inner, &event.data),
                "jobProgress" => handle_job_progress(&inner, &event.data),
                "jobETA" => handle_job_eta(&inner, &event.data),
                "jobResult" => handle_job_result(&inner, &event.data).await,
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
                        if api.sync("authenticated").await.is_err() {
                            tracing::warn!("project recovery failed");
                        }
                    });
                }
                _ => {}
            }
        }
    });
}

fn handle_job_retry(inner: &ProjectsInner, data: &Value) {
    // Retry diagnostics describe the abandoned attempt. Keep them internal so
    // public job consumers do not fail the render that is still being retried.
    if let Some(project) = state::project_by_id(inner, data) {
        project.retry_job(data);
    }
}

#[cfg(test)]
mod retry_tests;

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
        let supported = match api.get_supported_models(false).await {
            Ok(models) => models,
            Err(_) => {
                tracing::warn!("failed to resolve live model metadata");
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
