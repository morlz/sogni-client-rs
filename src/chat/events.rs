use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use serde_json::{Value, json};

use super::{
    api::ChatInner,
    types::{ActiveChat, ChatChunk, completion_from_state},
};
use crate::{ChatError, event::EventBus};
mod transport_recovery;
pub(super) use transport_recovery::{TransportRecovery, submitted, transport_error};

const CHAT_EVENT_LAG_CODE: &str = "CHAT_EVENT_STREAM_LAGGED";
const CHAT_EVENT_LAG_TYPE: &str = "event_stream_lagged";
const CHAT_EVENT_LAG_MESSAGE: &str =
    "chat event stream lagged; completion state could not be recovered";

pub(super) fn listen_for_chat_events(inner: &Arc<ChatInner>) {
    let mut receiver = inner.client.subscribe();
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        loop {
            let event = match receiver.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "chat event receiver lagged");
                    let Some(inner) = weak.upgrade() else {
                        return;
                    };
                    fail_active_chats_after_lag(&inner);
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            let Some(inner) = weak.upgrade() else {
                return;
            };
            match event.name.as_str() {
                "swarmLLMModels" => handle_models(&inner, &event.data),
                "jobTokens" => handle_tokens(&inner, &event.data),
                "llmJobResult" => handle_result(&inner, &event.data),
                "llmJobError" => handle_error(&inner, &event.data),
                "jobState" => handle_state(&inner, &event.data),
                "connecting" => transport_recovery::lost(&inner),
                "disconnected" => transport_recovery::closed(&inner, &event.data),
                "authenticated" => transport_recovery::authenticated(&inner, &event.data),
                _ => {}
            }
        }
    });
}

fn fail_active_chats_after_lag(inner: &ChatInner) {
    let affected = take_active_snapshot(&inner.active);
    fail_lagged_snapshot(&inner.events, affected);
}

fn take_active_snapshot(
    active: &parking_lot::RwLock<HashMap<String, ActiveChat>>,
) -> HashMap<String, ActiveChat> {
    std::mem::take(&mut *active.write())
}

fn fail_lagged_snapshot(events: &EventBus, affected: HashMap<String, ActiveChat>) {
    for (job_id, stream) in affected {
        let error = lagged_chat_error(&job_id);
        {
            let mut state = stream.state.write();
            state.error = Some(error.clone());
            state.complete = true;
            state.tool_calls.clear();
        }
        let _ = stream.sender.send(Err(error.clone().into()));
        stream.changed.notify_waiters();
        events.emit(
            "error",
            json!({
                "jobID": job_id,
                "error": error.error_type,
                "errorCode": error.code,
                "message": error.message,
            }),
        );
    }
}

fn lagged_chat_error(job_id: &str) -> ChatError {
    ChatError::from_payload(
        json!({
            "code": CHAT_EVENT_LAG_CODE,
            "type": CHAT_EVENT_LAG_TYPE,
            "message": CHAT_EVENT_LAG_MESSAGE,
        }),
        None,
        Some(job_id.to_owned()),
    )
}

fn handle_models(inner: &ChatInner, data: &Value) {
    let Some(models) = data.as_object() else {
        return;
    };
    let models: HashMap<String, Value> = models
        .iter()
        .map(|(id, value)| {
            let value = if value.is_number() {
                json!({"workers": value})
            } else {
                value.clone()
            };
            (id.clone(), value)
        })
        .collect();
    *inner.models.write() = models.clone();
    inner.events.emit("modelsUpdated", json!(models));
}

fn handle_tokens(inner: &ChatInner, data: &Value) {
    let Some(job_id) = data.get("jobID").and_then(Value::as_str) else {
        return;
    };
    transport_recovery::alive(inner, job_id);
    let mut active = inner.active.write();
    let Some(stream) = active.get_mut(job_id) else {
        return;
    };
    let chunk = ChatChunk {
        job_id: job_id.to_owned(),
        content: data
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        role: data
            .get("role")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        finish_reason: data
            .get("finishReason")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        usage: data.get("usage").cloned(),
        tool_calls: data
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    };
    {
        let mut state = stream.state.write();
        state.content.push_str(&chunk.content);
        if let Some(role) = &chunk.role {
            state.role.clone_from(role);
        }
        if chunk.finish_reason.is_some() {
            state.finish_reason.clone_from(&chunk.finish_reason);
        }
        if chunk.usage.is_some() {
            state.usage.clone_from(&chunk.usage);
        }
        for delta in &chunk.tool_calls {
            merge_tool_call_delta(&mut state.tool_calls, delta);
        }
    }
    let _ = stream.sender.send(Ok(chunk.clone()));
    stream.changed.notify_waiters();
    inner
        .events
        .emit("token", serde_json::to_value(chunk).unwrap_or(Value::Null));
}

fn handle_result(inner: &ChatInner, data: &Value) {
    let Some(job_id) = data.get("jobID").and_then(Value::as_str) else {
        return;
    };
    transport_recovery::alive(inner, job_id);
    let Some(stream) = inner.active.write().remove(job_id) else {
        return;
    };
    {
        let mut state = stream.state.write();
        state.complete = true;
        state.time_taken = data.get("timeTaken").and_then(Value::as_f64).unwrap_or(0.0);
        state.usage = data.get("usage").cloned().or_else(|| state.usage.clone());
        state.worker_name = data
            .get("workerName")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| state.worker_name.clone());
        state.cost = data.get("cost").cloned();
    }
    stream.changed.notify_waiters();
    drop(stream.sender);
    if let Some(result) = completion_from_state(job_id, &stream.state.read()) {
        inner.events.emit(
            "completed",
            serde_json::to_value(result).unwrap_or(Value::Null),
        );
    }
}

fn handle_error(inner: &ChatInner, data: &Value) {
    let Some(job_id) = data.get("jobID").and_then(Value::as_str) else {
        return;
    };
    transport_recovery::alive(inner, job_id);
    let Some(stream) = inner.active.write().remove(job_id) else {
        return;
    };
    let error = ChatError::from_payload(data.clone(), None, Some(job_id.to_owned()));
    {
        let mut state = stream.state.write();
        state.error = Some(error.clone());
        state.complete = true;
        state.tool_calls.clear();
    }
    let _ = stream.sender.send(Err(error.clone().into()));
    stream.changed.notify_waiters();
    inner.events.emit(
        "error",
        json!({
            "jobID": job_id,
            "error": error.error_type,
            "errorCode": error.code,
            "message": error.message,
            "workerName": data.get("workerName"),
        }),
    );
}

fn handle_state(inner: &ChatInner, data: &Value) {
    let Some(job_id) = data.get("jobID").and_then(Value::as_str) else {
        return;
    };
    transport_recovery::alive(inner, job_id);
    let active = inner.active.read();
    let Some(stream) = active.get(job_id) else {
        return;
    };
    if let Some(worker_name) = data.get("workerName").and_then(Value::as_str) {
        stream.state.write().worker_name = Some(worker_name.to_owned());
    }
    inner.events.emit(
        "jobState",
        json!({
            "jobID": job_id,
            "type": data.get("type"),
            "workerName": data.get("workerName"),
            "queuePosition": data.get("queuePosition"),
            "modelId": data.get("modelId"),
            "estimatedCost": data.get("estimatedCost"),
        }),
    );
}

fn merge_tool_call_delta(calls: &mut BTreeMap<usize, Value>, delta: &Value) {
    let index = delta.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let current = calls.entry(index).or_insert_with(|| {
        json!({
            "id": "",
            "type": "function",
            "function": {"name": "", "arguments": ""},
        })
    });
    if let Some(id) = delta
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        current["id"] = json!(id);
    }
    if let Some(name) = delta
        .pointer("/function/name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    {
        current["function"]["name"] = json!(name);
    }
    if let Some(arguments) = delta
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .filter(|arguments| !arguments.is_empty())
    {
        let combined = format!(
            "{}{}",
            current["function"]["arguments"].as_str().unwrap_or(""),
            arguments
        );
        current["function"]["arguments"] = json!(combined);
    }
}

#[cfg(test)]
mod tests;
