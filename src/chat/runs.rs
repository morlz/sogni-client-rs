use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use super::{
    api::ChatApi,
    validation::{
        alias, assert_chat_run_external_media, insert_header, map_chat_error, require_nonempty,
        required_str, run_field,
    },
};
use crate::{
    Error, Result,
    transport::SseStream,
    utils::{drop_nulls, new_id, path_segment},
};

impl ChatApi {
    pub async fn create_run(&self, params: &Value) -> Result<Value> {
        assert_chat_run_external_media(params)?;
        let messages = params
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::InvalidInput("messages must be an array".into()))?;
        let app_source = alias(params, "appSource", "app_source")
            .and_then(Value::as_str)
            .or_else(|| self.inner.client.app_source());
        let body = drop_nulls(json!({
            "messages": messages,
            "tools": params.get("tools"),
            "tool_choice": alias(params, "toolChoice", "tool_choice"),
            "model": params.get("model"),
            "sampling": params.get("sampling"),
            "media_references": alias(params, "mediaReferences", "media_references"),
            "media_context": alias(params, "mediaContext", "media_context"),
            "max_estimated_capacity_units": alias(params, "maxEstimatedCapacityUnits", "max_estimated_capacity_units"),
            "confirm_cost": alias(params, "confirmCost", "confirm_cost"),
            "session_id": alias(params, "sessionId", "session_id"),
            "client_message_id": alias(params, "clientMessageId", "client_message_id"),
            "token_type": alias(params, "tokenType", "token_type"),
            "billing_mode": alias(params, "billingMode", "billing_mode"),
            "app_source": app_source,
            "runtime_config": alias(params, "runtimeConfig", "runtime_config"),
        }));
        let operation_id = alias(params, "idempotencyKey", "idempotency_key")
            .and_then(Value::as_str)
            .map_or_else(new_id, ToOwned::to_owned);
        let mut headers = self.attribution_headers(params, app_source, &operation_id)?;
        if let Some(key) =
            alias(params, "idempotencyKey", "idempotency_key").and_then(Value::as_str)
        {
            insert_header(&mut headers, "Idempotency-Key", key)?;
        }
        let response = map_chat_error(
            self.inner
                .client
                .rest
                .post_with("/v1/chat/runs", &body, headers, None)
                .await,
        )?;
        run_field(&response)
    }

    pub async fn get_run(&self, run_id: &str) -> Result<Value> {
        require_nonempty(run_id, "run_id")?;
        let response = map_chat_error(
            self.inner
                .client
                .rest
                .get(&format!("/v1/chat/runs/{}", path_segment(run_id)), None)
                .await,
        )?;
        run_field(&response)
    }

    pub async fn cancel_run(&self, run_id: &str, reason: Option<&str>) -> Result<Value> {
        require_nonempty(run_id, "run_id")?;
        let body = reason.map_or_else(|| json!({}), |reason| json!({"reason": reason}));
        let response = map_chat_error(
            self.inner
                .client
                .rest
                .post(
                    &format!("/v1/chat/runs/{}/cancel", path_segment(run_id)),
                    &body,
                )
                .await,
        )?;
        run_field(&response)
    }

    pub async fn confirm_run_cost(&self, run_id: &str, params: &Value) -> Result<Value> {
        require_nonempty(run_id, "run_id")?;
        required_str(params, "toolCallId")?;
        required_str(params, "decision")?;
        let body = drop_nulls(json!({
            "tool_call_id": params.get("toolCallId"),
            "decision": params.get("decision"),
            "overrides": params.get("overrides"),
            "reason": params.get("reason"),
        }));
        let response = map_chat_error(
            self.inner
                .client
                .rest
                .post(
                    &format!("/v1/chat/runs/{}/confirm-cost", path_segment(run_id)),
                    &body,
                )
                .await,
        )?;
        run_field(&response)
    }

    pub async fn stream_run_events(
        &self,
        run_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<SseStream> {
        require_nonempty(run_id, "run_id")?;
        let mut headers = HeaderMap::new();
        if let Some(last_event_id) = last_event_id {
            insert_header(&mut headers, "Last-Event-ID", last_event_id)?;
        }
        self.inner
            .client
            .rest
            .stream_sse(
                &format!("/v1/chat/runs/{}/events/stream", path_segment(run_id)),
                None,
                headers,
            )
            .await
    }
}
