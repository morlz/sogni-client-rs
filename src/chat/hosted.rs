use serde_json::{Value, json};

use super::{
    api::{CHAT_TIMEOUT, ChatApi},
    media::normalize_vision_messages,
    tools::HOSTED_TOOL_NAMES,
    validation::{alias, map_chat_error, normalize_sogni_tools, required_str},
};
use crate::{
    Error, Result,
    utils::{drop_nulls, new_id},
};

impl ChatApi {
    /// OpenAI-compatible hosted chat completion. Streaming is intentionally rejected.
    pub async fn create_hosted_completion(&self, params: &Value) -> Result<Value> {
        if params.get("stream").and_then(Value::as_bool) == Some(true) {
            return Err(Error::InvalidInput(
                "hosted chat currently supports non-streaming requests only".into(),
            ));
        }
        let messages = params
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::InvalidInput("messages must be an array".into()))?;
        let messages = normalize_vision_messages(messages).await?;
        let app_source = alias(params, "appSource", "app_source")
            .and_then(Value::as_str)
            .or_else(|| self.inner.client.app_source());
        let template_kwargs = alias(params, "chatTemplateKwargs", "chat_template_kwargs")
            .cloned()
            .or_else(|| {
                params
                    .get("think")
                    .and_then(Value::as_bool)
                    .map(|think| json!({"enable_thinking": think}))
            });
        let body = drop_nulls(json!({
            "model": required_str(params, "model")?,
            "messages": messages,
            "app_source": app_source,
            "max_tokens": alias(params, "maxTokens", "max_tokens"),
            "temperature": params.get("temperature"),
            "top_p": alias(params, "topP", "top_p"),
            "top_k": alias(params, "topK", "top_k"),
            "min_p": alias(params, "minP", "min_p"),
            "repetition_penalty": alias(params, "repetitionPenalty", "repetition_penalty"),
            "frequency_penalty": alias(params, "frequencyPenalty", "frequency_penalty"),
            "presence_penalty": alias(params, "presencePenalty", "presence_penalty"),
            "stop": params.get("stop"),
            "token_type": alias(params, "tokenType", "token_type"),
            "billingMode": alias(params, "billingMode", "billing_mode"),
            "tools": params.get("tools"),
            "tool_choice": alias(params, "toolChoice", "tool_choice"),
            "sogni_tools": normalize_sogni_tools(alias(params, "sogniTools", "sogni_tools")),
            "sogni_tool_execution": alias(params, "sogniToolExecution", "sogni_tool_execution"),
            "task_profile": alias(params, "taskProfile", "task_profile"),
            "media_references": alias(params, "mediaReferences", "media_references"),
            "api_media_references": alias(params, "apiMediaReferences", "api_media_references"),
            "safe_content_filter": alias(params, "safeContentFilter", "safe_content_filter"),
            "chat_template_kwargs": template_kwargs,
            "response_format": alias(params, "responseFormat", "response_format"),
        }));
        let headers = self.attribution_headers(params, app_source, &new_id())?;
        map_chat_error(
            self.inner
                .client
                .rest
                .post_with("/v1/chat/completions", &body, headers, Some(CHAT_TIMEOUT))
                .await,
        )
    }

    pub async fn execute_hosted_tool(&self, params: &Value) -> Result<Value> {
        let tool = required_str(params, "tool")?;
        if !HOSTED_TOOL_NAMES.contains(&tool) {
            return Err(Error::InvalidInput(format!("unknown Sogni tool {tool}")));
        }
        let app_source = alias(params, "appSource", "app_source")
            .and_then(Value::as_str)
            .or_else(|| self.inner.client.app_source());
        let body = drop_nulls(json!({
            "tool": tool,
            "arguments": params.get("arguments").cloned().unwrap_or_else(|| json!({})),
            "app_source": app_source,
            "token_type": alias(params, "tokenType", "token_type"),
            "safe_content_filter": alias(params, "safeContentFilter", "safe_content_filter"),
        }));
        let headers = self.attribution_headers(params, app_source, &new_id())?;
        map_chat_error(
            self.inner
                .client
                .rest
                .post_with(
                    "/v1/creative-agent/tools/execute",
                    &body,
                    headers,
                    Some(CHAT_TIMEOUT),
                )
                .await,
        )
    }
}
