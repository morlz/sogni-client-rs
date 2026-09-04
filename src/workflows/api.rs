use std::sync::Arc;

use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use super::{
    templates::CreativeWorkflowTemplatesApi,
    types::{
        ReseedWorkflowMetadata, ReseedWorkflowResult, ResumeWorkflowResult, TERMINAL_STATUSES,
        WorkflowBillingOptions, WorkflowStart,
    },
    validation::{
        assert_external_media, insert_header, require_id, require_object, workflow_field,
    },
};
use crate::{
    Error, Result,
    transport::{ApiClient, SseStream},
    utils::{new_id, path_segment},
};

/// Durable, deterministic multi-step creative workflow operations.
#[derive(Clone)]
pub struct CreativeWorkflowsApi {
    client: Arc<ApiClient>,
    pub templates: CreativeWorkflowTemplatesApi,
}

impl std::fmt::Debug for CreativeWorkflowsApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreativeWorkflowsApi")
            .finish_non_exhaustive()
    }
}

impl CreativeWorkflowsApi {
    pub(crate) fn new(client: Arc<ApiClient>) -> Self {
        Self {
            templates: CreativeWorkflowTemplatesApi::new(client.clone()),
            client,
        }
    }

    pub async fn start(&self, options: WorkflowStart) -> Result<Value> {
        match (options.input.is_some(), options.workflow_id.is_some()) {
            (true, true) => {
                return Err(Error::InvalidInput(
                    "workflow start accepts input or workflow_id, not both".into(),
                ));
            }
            (false, false) => {
                return Err(Error::InvalidInput(
                    "workflow start requires input or workflow_id".into(),
                ));
            }
            _ => {}
        }
        if let Some(references) = &options.media_references {
            assert_external_media(references)?;
        }
        let app_source = options
            .app_source
            .as_deref()
            .or_else(|| self.client.app_source());
        let workload = self
            .client
            .resolve_workload_attribution(options.attribution.as_ref(), Some(&new_id()));
        let mut headers = self
            .client
            .attribution_headers(app_source, workload.as_ref())?;
        if let Some(key) = options.idempotency_key.as_deref() {
            insert_header(&mut headers, "Idempotency-Key", key)?;
        }
        let mut body = serde_json::to_value(&options)?;
        if let Some(source) = app_source {
            body["app_source"] = json!(source);
        }
        let response = self
            .client
            .rest
            .post_with("/v1/creative-agent/workflows", &body, headers, None)
            .await?;
        workflow_field(&response, "workflow")
    }

    pub async fn list(&self, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<Value>> {
        let query = json!({"limit": limit, "offset": offset});
        let response = self
            .client
            .rest
            .get("/v1/creative-agent/workflows", Some(&query))
            .await?;
        Ok(workflow_field(&response, "workflows")?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    pub async fn get(&self, workflow_id: &str) -> Result<Value> {
        self.get_field(workflow_id, "", "workflow").await
    }

    pub async fn events(&self, workflow_id: &str) -> Result<Vec<Value>> {
        Ok(self
            .get_field(workflow_id, "/events", "events")
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    pub async fn cancel(&self, workflow_id: &str) -> Result<Value> {
        self.post_action(workflow_id, "cancel", &json!({}), HeaderMap::new())
            .await
    }

    pub async fn confirm_cost(&self, workflow_id: &str, body: &Value) -> Result<Value> {
        require_object(body, "confirm_cost body")?;
        self.post_action(workflow_id, "confirm-cost", body, HeaderMap::new())
            .await
    }

    pub async fn resume(
        &self,
        workflow_id: &str,
        options: WorkflowBillingOptions,
    ) -> Result<ResumeWorkflowResult> {
        let response = self
            .billing_action_response(workflow_id, "resume", options, None)
            .await?;
        resume_result(&response)
    }

    pub async fn reseed(
        &self,
        workflow_id: &str,
        options: WorkflowBillingOptions,
        seed_overrides: Option<Value>,
    ) -> Result<ReseedWorkflowResult> {
        let response = self
            .billing_action_response(workflow_id, "reseed", options, seed_overrides)
            .await?;
        reseed_result(&response)
    }

    /// Open a resumable SSE stream. Set `last_event_id` after reconnecting.
    pub async fn stream_events(
        &self,
        workflow_id: &str,
        after: Option<&str>,
        last_event_id: Option<&str>,
    ) -> Result<SseStream> {
        require_id(workflow_id, "workflow_id")?;
        let cursor = after.or(last_event_id);
        let query = cursor.map(|cursor| json!({"after": cursor}));
        let mut headers = HeaderMap::new();
        if let Some(last_event_id) = last_event_id {
            insert_header(&mut headers, "Last-Event-ID", last_event_id)?;
        }
        self.client
            .rest
            .stream_sse(
                &format!(
                    "/v1/creative-agent/workflows/{}/events/stream",
                    path_segment(workflow_id)
                ),
                query.as_ref(),
                headers,
            )
            .await
    }

    #[must_use]
    pub fn is_terminal_status(status: &str) -> bool {
        TERMINAL_STATUSES.contains(&status)
    }

    async fn get_field(&self, id: &str, suffix: &str, field: &str) -> Result<Value> {
        require_id(id, "workflow_id")?;
        let response = self
            .client
            .rest
            .get(
                &format!(
                    "/v1/creative-agent/workflows/{}{}",
                    path_segment(id),
                    suffix
                ),
                None,
            )
            .await?;
        workflow_field(&response, field)
    }

    async fn billing_action_response(
        &self,
        workflow_id: &str,
        action: &str,
        options: WorkflowBillingOptions,
        seed_overrides: Option<Value>,
    ) -> Result<Value> {
        let app_source = options
            .app_source
            .as_deref()
            .or_else(|| self.client.app_source());
        let workload = self
            .client
            .resolve_workload_attribution(options.attribution.as_ref(), Some(&new_id()));
        let headers = self
            .client
            .attribution_headers(app_source, workload.as_ref())?;
        let mut body = serde_json::to_value(&options)?;
        if let Some(source) = app_source {
            body["app_source"] = json!(source);
        }
        if let Some(seed_overrides) = seed_overrides {
            body["seed_overrides"] = seed_overrides;
        }
        self.post_action_response(workflow_id, action, &body, headers)
            .await
    }

    async fn post_action(
        &self,
        workflow_id: &str,
        action: &str,
        body: &Value,
        headers: HeaderMap,
    ) -> Result<Value> {
        let response = self
            .post_action_response(workflow_id, action, body, headers)
            .await?;
        workflow_field(&response, "workflow")
    }

    async fn post_action_response(
        &self,
        workflow_id: &str,
        action: &str,
        body: &Value,
        headers: HeaderMap,
    ) -> Result<Value> {
        require_id(workflow_id, "workflow_id")?;
        self.client
            .rest
            .post_with(
                &format!(
                    "/v1/creative-agent/workflows/{}/{}",
                    path_segment(workflow_id),
                    action
                ),
                body,
                headers,
                None,
            )
            .await
    }
}

fn resume_result(response: &Value) -> Result<ResumeWorkflowResult> {
    Ok(ResumeWorkflowResult {
        workflow: workflow_field(response, "workflow")?,
        resumed: response.pointer("/data/resumed").and_then(Value::as_bool) == Some(true),
    })
}

fn reseed_result(response: &Value) -> Result<ReseedWorkflowResult> {
    let workflow = workflow_field(response, "workflow")?;
    let reseed = response.pointer("/data/reseed");
    let cloned_from_run_id = reseed
        .and_then(|value| value.get("cloned_from_run_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let steps = reseed
        .and_then(|value| value.get("steps"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(ReseedWorkflowResult {
        workflow,
        reseed: ReseedWorkflowMetadata {
            cloned_from_run_id,
            steps,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_failure_is_terminal() {
        assert!(CreativeWorkflowsApi::is_terminal_status("partial_failure"));
    }

    #[test]
    fn resume_result_preserves_action_metadata() {
        let result = resume_result(&json!({
            "data": {
                "workflow": {"workflowId": "wf-1", "status": "running"},
                "resumed": true,
            }
        }))
        .expect("resume envelope");

        assert_eq!(result.workflow["workflowId"], "wf-1");
        assert!(result.resumed);
    }

    #[test]
    fn reseed_result_preserves_source_and_step_seeds() {
        let steps = json!([
            {"id": "image", "seed": 123},
            {"id": "video", "seed": 456, "futureMetadata": {"kept": true}},
        ]);
        let result = reseed_result(&json!({
            "data": {
                "workflow": {"workflowId": "wf-2", "status": "queued"},
                "reseed": {
                    "cloned_from_run_id": "wf-1",
                    "steps": steps,
                },
            }
        }))
        .expect("reseed envelope");

        assert_eq!(result.workflow["workflowId"], "wf-2");
        assert_eq!(result.reseed.cloned_from_run_id, "wf-1");
        assert_eq!(result.reseed.steps, steps.as_array().unwrap().clone());
    }
}
