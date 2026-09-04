use std::{collections::HashSet, future::Future};

use serde_json::{Value, json};

use super::{
    api::ChatApi,
    tools::HOSTED_TOOL_NAMES,
    types::{
        ChatAutoToolCancellation, ChatAutoToolOptions, ChatCompletion, ChatToolCall,
        ChatToolExecutionResult, ChatToolHistoryEntry,
    },
    validation::{parse_attribution, reject_untyped_tool_controls, require_object},
};
use crate::{Error, OperationScope, Result, WorkloadAttribution, utils::new_id};

const MAX_AUTO_TOOL_ROUNDS: usize = 32;

impl ChatApi {
    /// Run a bounded, non-streaming completion loop for caller-owned custom tools.
    ///
    /// Hosted and Sogni tools are deliberately not dispatched by this method. Use
    /// hosted chat, durable chat runs, or explicit project APIs for those tools.
    pub async fn create_completion_with_custom_tools(
        &self,
        params: &Value,
        options: ChatAutoToolOptions,
    ) -> Result<ChatCompletion> {
        reject_untyped_tool_controls(params)?;
        require_non_streaming(params)?;
        let (logical_attribution, child_attribution) = self.auto_tool_attributions(params)?;
        let chat = self.clone();
        drive_auto_tool_loop(
            params.clone(),
            options,
            move |mut round_params, round, cancel| {
                let chat = chat.clone();
                set_round_attribution(
                    &mut round_params,
                    if round == 0 {
                        logical_attribution.clone()
                    } else {
                        child_attribution.clone()
                    },
                );
                async move {
                    chat.create_single_completion(&round_params, cancel.as_ref())
                        .await
                }
            },
        )
        .await
    }

    fn auto_tool_attributions(&self, params: &Value) -> Result<(Option<Value>, Option<Value>)> {
        let requested = parse_attribution(params.get("attribution"))?;
        let logical = self
            .inner
            .client
            .resolve_workload_attribution(requested.as_ref(), Some(&new_id()));
        let child = logical.as_ref().and_then(auto_tool_child_attribution);
        Ok((
            logical.map(serde_json::to_value).transpose()?,
            child.map(serde_json::to_value).transpose()?,
        ))
    }
}

fn require_non_streaming(params: &Value) -> Result<()> {
    match params.get("stream") {
        None | Some(Value::Bool(false)) => Ok(()),
        Some(Value::Bool(true)) => Err(Error::InvalidInput(
            "automatic custom-tool execution is not supported with stream=true".into(),
        )),
        Some(_) => Err(Error::InvalidInput("stream must be a boolean".into())),
    }
}

fn auto_tool_child_attribution(logical: &WorkloadAttribution) -> Option<WorkloadAttribution> {
    let operation_id = logical.operation_id.clone()?;
    let mut child = logical.clone();
    child.operation_scope = Some(OperationScope::Child);
    child.operation_id = None;
    child.root_operation_id = Some(
        logical
            .root_operation_id
            .clone()
            .unwrap_or_else(|| operation_id.clone()),
    );
    child.parent_operation_id = Some(operation_id);
    Some(child)
}

fn set_round_attribution(params: &mut Value, attribution: Option<Value>) {
    let Some(params) = params.as_object_mut() else {
        return;
    };
    params.insert("stream".into(), Value::Bool(false));
    if let Some(attribution) = attribution {
        params.insert("attribution".into(), attribution);
    } else {
        params.remove("attribution");
    }
}

async fn drive_auto_tool_loop<C, Fut>(
    params: Value,
    options: ChatAutoToolOptions,
    mut complete: C,
) -> Result<ChatCompletion>
where
    C: FnMut(Value, usize, Option<ChatAutoToolCancellation>) -> Fut,
    Fut: Future<Output = Result<ChatCompletion>>,
{
    require_object(&params, "chat params")?;
    if !(1..=MAX_AUTO_TOOL_ROUNDS).contains(&options.max_tool_rounds) {
        return Err(Error::InvalidInput(format!(
            "max_tool_rounds must be between 1 and {MAX_AUTO_TOOL_ROUNDS}"
        )));
    }
    let mut messages = params
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| Error::InvalidInput("messages must be an array".into()))?;
    let mut history = Vec::new();

    for round in 0..options.max_tool_rounds {
        ensure_not_cancelled(options.cancellation.as_ref())?;
        let mut round_params = params.clone();
        round_params["messages"] = Value::Array(messages.clone());
        round_params["stream"] = Value::Bool(false);
        let mut completion = complete(round_params, round, options.cancellation.clone()).await?;
        if completion.finish_reason != "tool_calls" || completion.tool_calls.is_empty() {
            if !history.is_empty() {
                completion.tool_history = Some(history);
            }
            return Ok(completion);
        }

        validate_custom_tool_calls(&completion.tool_calls)?;
        let mut results = Vec::with_capacity(completion.tool_calls.len());
        for tool_call in &completion.tool_calls {
            results.push(execute_custom_tool(&options, tool_call.clone()).await?);
        }
        append_tool_messages(&mut messages, &completion, &completion.tool_calls, &results);
        history.push(ChatToolHistoryEntry {
            round,
            tool_calls: completion.tool_calls,
            tool_results: results,
        });
    }

    Err(Error::Protocol(format!(
        "maximum automatic tool rounds ({}) exceeded",
        options.max_tool_rounds
    )))
}

fn validate_custom_tool_calls(tool_calls: &[ChatToolCall]) -> Result<()> {
    let mut ids = HashSet::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        if tool_call.call_type != "function" {
            return Err(Error::Protocol(format!(
                "unsupported tool call type: {}",
                tool_call.call_type
            )));
        }
        if tool_call.id.trim().is_empty() || !ids.insert(tool_call.id.as_str()) {
            return Err(Error::Protocol(
                "tool call IDs must be non-empty and unique within each round".into(),
            ));
        }
        let name = tool_call.function.name.as_str();
        if name.trim().is_empty() || name.trim() != name {
            return Err(Error::Protocol(
                "tool call name must be non-empty and contain no surrounding whitespace".into(),
            ));
        }
        if name
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("sogni_"))
            || HOSTED_TOOL_NAMES
                .iter()
                .any(|hosted| hosted.eq_ignore_ascii_case(name))
        {
            return Err(Error::InvalidInput(format!(
                "tool {name} requires hosted chat, a durable chat run, or explicit project execution; automatic custom-tool execution will not run it client-side"
            )));
        }
    }
    Ok(())
}

async fn execute_custom_tool(
    options: &ChatAutoToolOptions,
    tool_call: ChatToolCall,
) -> Result<ChatToolExecutionResult> {
    ensure_not_cancelled(options.cancellation.as_ref())?;
    let execution = options.execute(tool_call.clone());
    let outcome = if let Some(cancellation) = &options.cancellation {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(auto_tool_cancelled()),
            result = execution => result,
        }
    } else {
        execution.await
    };
    let tool_name = tool_call.function.name;
    match outcome {
        Ok(content) => Ok(ChatToolExecutionResult {
            tool_call_id: tool_call.id,
            tool_name,
            success: true,
            result_urls: Vec::new(),
            content,
            error: None,
        }),
        Err(error) => {
            let error = error.to_string();
            Ok(ChatToolExecutionResult {
                tool_call_id: tool_call.id,
                tool_name,
                success: false,
                result_urls: Vec::new(),
                content: json!({"success": false, "error": error}).to_string(),
                error: Some(error),
            })
        }
    }
}

fn append_tool_messages(
    messages: &mut Vec<Value>,
    completion: &ChatCompletion,
    tool_calls: &[ChatToolCall],
    results: &[ChatToolExecutionResult],
) {
    messages.push(json!({
        "role": "assistant",
        "content": if completion.content.is_empty() {
            Value::Null
        } else {
            Value::String(completion.content.clone())
        },
        "tool_calls": tool_calls,
    }));
    messages.extend(tool_calls.iter().zip(results).map(|(tool_call, result)| {
        json!({
            "role": "tool",
            "content": result.content,
            "tool_call_id": tool_call.id,
            "name": tool_call.function.name,
        })
    }));
}

fn ensure_not_cancelled(cancellation: Option<&ChatAutoToolCancellation>) -> Result<()> {
    if cancellation.is_some_and(ChatAutoToolCancellation::is_cancelled) {
        Err(auto_tool_cancelled())
    } else {
        Ok(())
    }
}

pub(super) fn auto_tool_cancelled() -> Error {
    Error::Transport("automatic custom-tool execution was cancelled".into())
}

#[cfg(test)]
mod tests;
