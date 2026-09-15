use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};

use super::{
    api::{CHAT_TIMEOUT, ChatApi},
    auto_tools::auto_tool_cancelled,
    media::{normalize_vision_messages, redact_inline_images},
    types::{ActiveChat, ChatAutoToolCancellation, ChatCompletion, ChatStream, ChatStreamState},
    validation::{
        alias, normalize_sogni_tools, parse_attribution, reject_untyped_tool_controls,
        require_object, required_str,
    },
};
use crate::{
    Error, Result,
    utils::{drop_nulls, new_id, path_segment},
};

impl ChatApi {
    /// Create one non-streaming socket-native completion.
    pub async fn create_completion(&self, params: &Value) -> Result<ChatCompletion> {
        reject_untyped_tool_controls(params)?;
        self.create_single_completion(params, None).await
    }

    pub(super) async fn create_single_completion(
        &self,
        params: &Value,
        cancellation: Option<&ChatAutoToolCancellation>,
    ) -> Result<ChatCompletion> {
        if cancellation.is_some_and(ChatAutoToolCancellation::is_cancelled) {
            return Err(auto_tool_cancelled());
        }
        let stream = self.start_completion(params, false).await?;
        let job_id = stream.job_id.clone();
        let result = if let Some(cancellation) = cancellation {
            tokio::select! {
                biased;
                () = cancellation.cancelled() => Err(auto_tool_cancelled()),
                result = stream.wait(Some(CHAT_TIMEOUT)) => result,
            }
        } else {
            stream.wait(Some(CHAT_TIMEOUT)).await
        };
        match result {
            Ok(completion) => Ok(completion),
            Err(error) => {
                self.inner.active.write().remove(&job_id);
                Err(error)
            }
        }
    }

    /// Start a streaming socket-native completion.
    pub async fn stream_completion(&self, params: &Value) -> Result<ChatStream> {
        reject_untyped_tool_controls(params)?;
        self.start_completion(params, true).await
    }

    async fn start_completion(&self, params: &Value, stream: bool) -> Result<ChatStream> {
        require_object(params, "chat params")?;
        let model = required_str(params, "model")?;
        let messages = params
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::InvalidInput("messages must be an array".into()))?;
        let messages = normalize_vision_messages(messages).await?;
        let job_id = new_id();
        let app_source = params
            .get("appSource")
            .or_else(|| params.get("app_source"))
            .and_then(Value::as_str)
            .or_else(|| self.inner.client.app_source());
        let attribution = parse_attribution(params.get("attribution"))?;
        let workload = self
            .inner
            .client
            .resolve_workload_attribution(attribution.as_ref(), Some(&job_id));
        let mut request = json!({
            "jobID": job_id,
            "type": "llm",
            "model": model,
            "messages": messages,
            "appSource": app_source,
            "max_tokens": alias(params, "maxTokens", "max_tokens"),
            "temperature": params.get("temperature"),
            "top_p": alias(params, "topP", "top_p"),
            "top_k": alias(params, "topK", "top_k"),
            "min_p": alias(params, "minP", "min_p"),
            "stream": stream,
            "repetition_penalty": alias(params, "repetitionPenalty", "repetition_penalty"),
            "frequency_penalty": alias(params, "frequencyPenalty", "frequency_penalty"),
            "presence_penalty": alias(params, "presencePenalty", "presence_penalty"),
            "stop": params.get("stop"),
            "tokenType": alias(params, "tokenType", "token_type"),
            "billingMode": params.get("billingMode"),
            "tools": params.get("tools"),
            "tool_choice": alias(params, "toolChoice", "tool_choice"),
            "sogni_tools": normalize_sogni_tools(alias(params, "sogniTools", "sogni_tools")),
            "sogni_tool_execution": alias(params, "sogniToolExecution", "sogni_tool_execution"),
            "taskProfile": alias(params, "taskProfile", "task_profile"),
            "response_format": alias(params, "responseFormat", "response_format"),
            "safeContentFilter": alias(params, "safeContentFilter", "safe_content_filter"),
        });
        if let Some(think) = params.get("think").and_then(Value::as_bool) {
            request["chat_template_kwargs"] = json!({"enable_thinking": think});
        }
        if let Some(workload) = workload {
            for (key, value) in workload.wire_fields() {
                request[key] = json!(value);
            }
        }
        request = drop_nulls(request);
        let (sender, receiver) = mpsc::unbounded_channel();
        let state = Arc::new(RwLock::new(ChatStreamState {
            role: "assistant".into(),
            ..ChatStreamState::default()
        }));
        let changed = Arc::new(Notify::new());
        let terminal_epoch = self.inner.recovery.lock().submitting(job_id.clone());
        self.inner.active.write().insert(
            job_id.clone(),
            ActiveChat {
                sender,
                state: state.clone(),
                changed: changed.clone(),
            },
        );
        if let Err(error) = self
            .inner
            .client
            .send_socket("llmJobRequest", &request)
            .await
        {
            self.inner.active.write().remove(&job_id);
            self.inner.recovery.lock().unsent.remove(&job_id);
            if matches!(error, Error::InvalidInput(_)) {
                return Err(error);
            }
            return Err(super::events::transport_error(&job_id, Some(&error)).into());
        }
        super::events::submitted(&self.inner, &job_id, terminal_epoch);
        Ok(ChatStream {
            job_id,
            receiver,
            state,
            changed,
        })
    }

    pub async fn estimate_cost(&self, params: &Value) -> Result<Value> {
        let model = required_str(params, "model")?;
        let messages = params
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::InvalidInput("messages must be an array".into()))?;
        let mut messages = normalize_vision_messages(messages).await?;
        redact_inline_images(&mut messages);
        let serialized = serde_json::to_string(&messages)?;
        let javascript_length = serialized.encode_utf16().count();
        let input_tokens = javascript_length.div_ceil(4);
        let max_output = alias(params, "maxTokens", "max_tokens")
            .and_then(Value::as_u64)
            .or_else(|| {
                let models = self.inner.models.read();
                let info = models.get(model)?;
                let complex = params.get("think").and_then(Value::as_bool) == Some(true)
                    && matches!(
                        params.get("taskProfile").and_then(Value::as_str),
                        Some("coding" | "reasoning")
                    );
                if complex {
                    info.pointer("/maxOutputTokens/thinkingComplexDefault")
                        .and_then(Value::as_u64)
                } else {
                    None
                }
                .or_else(|| {
                    info.pointer("/maxOutputTokens/default")
                        .and_then(Value::as_u64)
                })
            })
            .unwrap_or(4096);
        let token_type = alias(params, "tokenType", "token_type")
            .and_then(Value::as_str)
            .unwrap_or("sogni");
        let path = [
            token_type.to_owned(),
            model.to_owned(),
            input_tokens.to_string(),
            max_output.to_string(),
        ]
        .iter()
        .map(|part| path_segment(part))
        .collect::<Vec<_>>()
        .join("/");
        let response = self
            .inner
            .client
            .socket_get(&format!("/api/v1/job-llm/estimate/{path}"), None)
            .await?;
        let quote = response
            .get("quote")
            .ok_or_else(|| Error::Protocol("chat estimate response missing quote".into()))?;
        Ok(json!({
            "costInUSD": quote.get("costInUSD"),
            "costInSogni": quote.get("costInSogni"),
            "costInSpark": quote.get("costInSpark"),
            "costInToken": quote.get("costInToken"),
            "inputTokens": quote.get("inputTokens"),
            "outputTokens": quote.get("outputTokens"),
        }))
    }
}
