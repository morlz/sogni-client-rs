use std::sync::Arc;

use async_stream::try_stream;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::types::*;
use crate::{
    Error, Result,
    transport::ApiClient,
    utils::{new_id, path_segment},
};

/// Hosted World operations. Account authorization and billing stay on the server.
#[derive(Clone, Debug)]
pub struct WorldsApi {
    pub builds: WorldBuildsApi,
}

impl WorldsApi {
    pub(crate) fn new(client: Arc<ApiClient>) -> Self {
        Self {
            builds: WorldBuildsApi { client },
        }
    }
}

/// Start and observe a hosted build, explicitly answer its quote and review takes.
#[derive(Clone)]
pub struct WorldBuildsApi {
    client: Arc<ApiClient>,
}

impl std::fmt::Debug for WorldBuildsApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldBuildsApi").finish_non_exhaustive()
    }
}

impl WorldBuildsApi {
    pub async fn start(&self, params: StartWorldBuildParams) -> Result<WorldBuildRecord> {
        let workload = self
            .client
            .resolve_workload_attribution(params.attribution.as_ref(), Some(&new_id()));
        let mut headers = self
            .client
            .attribution_headers(self.client.app_source(), workload.as_ref())?;
        if let Some(key) = params
            .idempotency_key
            .as_deref()
            .filter(|key| !key.is_empty())
        {
            headers.insert("Idempotency-Key", header(key)?);
        }
        let mut body = serde_json::to_value(&params)?;
        // Optional strings follow the upstream truthy omission rule.
        if params.look.as_deref() == Some("") {
            body.as_object_mut()
                .expect("start params serialize as an object")
                .remove("look");
        }
        let response = self
            .client
            .rest
            .post_with("/v1/world-builds", &body, headers, None)
            .await?;
        field(&response, "run")
    }

    pub async fn list(&self, mut options: ListWorldBuildsOptions) -> Result<Vec<WorldBuildRecord>> {
        if options.world_id.as_deref() == Some("") {
            options.world_id = None;
        }
        let response = self
            .client
            .rest
            .get("/v1/world-builds", Some(&serde_json::to_value(options)?))
            .await?;
        field(&response, "runs")
    }

    pub async fn get(&self, run_id: &str) -> Result<WorldBuildRecord> {
        let response = self.client.rest.get(&run_path(run_id), None).await?;
        field(&response, "run")
    }

    pub async fn events(
        &self,
        run_id: &str,
        after_sequence: Option<u64>,
    ) -> Result<Vec<WorldBuildEvent>> {
        let query = after_sequence.map(|after| json!({"after": after}));
        let response = self
            .client
            .rest
            .get(&format!("{}/events", run_path(run_id)), query.as_ref())
            .await?;
        field(&response, "events")
    }

    /// Confirm, cancel or request a new quote only after an explicit caller decision.
    pub async fn confirm_cost(
        &self,
        run_id: &str,
        params: ConfirmWorldBuildCostParams,
    ) -> Result<WorldBuildRecord> {
        self.action(run_id, "confirm-cost", &serde_json::to_value(params)?)
            .await
    }

    pub async fn review(
        &self,
        run_id: &str,
        mut params: ReviewWorldBuildTaskParams,
    ) -> Result<WorldBuildRecord> {
        if params.note.as_deref() == Some("") {
            params.note = None;
        }
        self.action(run_id, "review", &serde_json::to_value(params)?)
            .await
    }

    pub async fn cancel(&self, run_id: &str, reason: Option<&str>) -> Result<WorldBuildRecord> {
        let body = reason
            .filter(|reason| !reason.is_empty())
            .map_or_else(|| json!({}), |reason| json!({"reason": reason}));
        self.action(run_id, "cancel", &body).await
    }

    /// Resumable event stream. Session changes, client closure and local cancellation
    /// stop it. Keep-alive frames, status snapshots and malformed events are skipped.
    pub async fn stream_events(
        &self,
        run_id: &str,
        options: StreamWorldBuildEventsOptions,
    ) -> Result<WorldBuildEventStream> {
        let mut headers = HeaderMap::new();
        if let Some(last_event_id) = options.last_event_id {
            headers.insert("Last-Event-ID", header(&last_event_id.to_string())?);
        }
        let path = format!("{}/events/stream", run_path(run_id));
        let open = self.client.rest.stream_sse(&path, None, headers);
        let mut events = if let Some(cancellation) = &options.cancellation {
            tokio::select! {
                biased;
                () = cancellation.cancelled() => return Err(cancelled()),
                events = open => events?,
            }
        } else {
            open.await?
        };
        let stream = try_stream! {
            loop {
                let next = if let Some(cancellation) = &options.cancellation {
                    tokio::select! {
                        biased;
                        () = cancellation.cancelled() => Err(cancelled()),
                        event = events.next() => Ok(event),
                    }
                } else {
                    Ok(events.next().await)
                }?;
                let Some(event) = next else { break; };
                let event = event?;
                if event.event == "run_status" || !event.data.is_object() {
                    continue;
                }
                if let Ok(event) = serde_json::from_value::<WorldBuildEvent>(event.data) {
                    yield event;
                }
            }
        };
        Ok(Box::pin(stream))
    }

    async fn action(&self, run_id: &str, action: &str, body: &Value) -> Result<WorldBuildRecord> {
        let response = self
            .client
            .rest
            .post(&format!("{}/{action}", run_path(run_id)), body)
            .await?;
        field(&response, "run")
    }
}

fn run_path(run_id: &str) -> String {
    format!("/v1/world-builds/{}", path_segment(run_id))
}

fn field<T: DeserializeOwned>(response: &Value, field: &str) -> Result<T> {
    let value = response
        .get("data")
        .and_then(|data| data.get(field))
        .ok_or_else(|| Error::Protocol(format!("world build response missing data.{field}")))?;
    serde_json::from_value(value.clone())
        .map_err(|error| Error::Protocol(format!("invalid world build {field}: {error}")))
}

fn header(value: &str) -> Result<HeaderValue> {
    HeaderValue::from_str(value)
        .map_err(|error| Error::InvalidInput(format!("invalid world build header: {error}")))
}

fn cancelled() -> Error {
    Error::Transport("world build event stream was cancelled".into())
}
