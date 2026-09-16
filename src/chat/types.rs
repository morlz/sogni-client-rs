use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures_util::Stream;
use parking_lot::RwLock;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{ChatError, Error, Result};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ChatToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default = "function_type")]
    pub call_type: String,
    #[serde(default)]
    pub function: ChatToolFunction,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ChatToolFunction {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

fn function_type() -> String {
    "function".into()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatChunk {
    #[serde(rename = "jobID", alias = "jobId", alias = "job_id")]
    pub job_id: String,
    #[serde(default)]
    pub content: String,
    pub role: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Option<Value>,
    #[serde(
        default,
        rename = "tool_calls",
        alias = "toolCalls",
        deserialize_with = "deserialize_null_default"
    )]
    pub tool_calls: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatCompletion {
    #[serde(rename = "jobID", alias = "jobId", alias = "job_id")]
    pub job_id: String,
    pub content: String,
    pub role: String,
    pub finish_reason: String,
    pub usage: Value,
    pub time_taken: f64,
    pub worker_name: Option<String>,
    pub cost: Option<Value>,
    #[serde(
        default,
        rename = "tool_calls",
        alias = "toolCalls",
        deserialize_with = "deserialize_null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tool_calls: Vec<ChatToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_history: Option<Vec<ChatToolHistoryEntry>>,
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatToolExecutionResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub success: bool,
    #[serde(default)]
    pub result_urls: Vec<String>,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatToolHistoryEntry {
    pub round: usize,
    pub tool_calls: Vec<ChatToolCall>,
    pub tool_results: Vec<ChatToolExecutionResult>,
}

/// Cooperative cancellation handle for an automatic custom-tool loop.
#[derive(Clone, Debug, Default)]
pub struct ChatAutoToolCancellation {
    inner: CancellationToken,
}

impl ChatAutoToolCancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    pub(super) async fn cancelled(&self) {
        self.inner.cancelled().await;
    }
}

type ChatToolFuture = Pin<Box<dyn Future<Output = Result<String>> + Send + 'static>>;
type ChatToolHandler = dyn Fn(ChatToolCall) -> ChatToolFuture + Send + Sync + 'static;

/// Typed controls for non-streaming automatic custom-tool execution.
#[derive(Clone)]
pub struct ChatAutoToolOptions {
    pub(super) max_tool_rounds: usize,
    pub(super) on_tool_call: Arc<ChatToolHandler>,
    pub(super) cancellation: Option<ChatAutoToolCancellation>,
}

impl ChatAutoToolOptions {
    /// Configure the asynchronous handler for caller-defined function tools.
    ///
    /// Handler failures are returned to the model as failed tool results, matching
    /// the sibling SDKs. Hosted/Sogni tool names are rejected before this callback.
    #[must_use]
    pub fn new<F, Fut>(on_tool_call: F) -> Self
    where
        F: Fn(ChatToolCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String>> + Send + 'static,
    {
        Self {
            max_tool_rounds: 5,
            on_tool_call: Arc::new(move |tool_call| Box::pin(on_tool_call(tool_call))),
            cancellation: None,
        }
    }

    /// Set the round limit. Values outside `1..=32` are rejected before dispatch.
    #[must_use]
    pub fn max_tool_rounds(mut self, max_tool_rounds: usize) -> Self {
        self.max_tool_rounds = max_tool_rounds;
        self
    }

    /// Stop the loop cooperatively, including while a callback is awaiting.
    #[must_use]
    pub fn cancellation(mut self, cancellation: ChatAutoToolCancellation) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub(super) async fn execute(&self, tool_call: ChatToolCall) -> Result<String> {
        (self.on_tool_call)(tool_call).await
    }
}

impl std::fmt::Debug for ChatAutoToolOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatAutoToolOptions")
            .field("max_tool_rounds", &self.max_tool_rounds)
            .field("on_tool_call", &"<callback>")
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

#[derive(Default)]
pub(super) struct ChatStreamState {
    pub(super) content: String,
    pub(super) role: String,
    pub(super) finish_reason: Option<String>,
    pub(super) usage: Option<Value>,
    pub(super) time_taken: f64,
    pub(super) worker_name: Option<String>,
    pub(super) cost: Option<Value>,
    pub(super) tool_calls: BTreeMap<usize, Value>,
    pub(super) complete: bool,
    pub(super) error: Option<ChatError>,
}

pub(super) struct ActiveChat {
    pub(super) session: u64,
    pub(super) sender: mpsc::UnboundedSender<Result<ChatChunk>>,
    pub(super) state: Arc<RwLock<ChatStreamState>>,
    pub(super) changed: Arc<Notify>,
}

/// Async stream returned by socket-native chat completions.
pub struct ChatStream {
    pub(super) job_id: String,
    pub(super) receiver: mpsc::UnboundedReceiver<Result<ChatChunk>>,
    pub(super) state: Arc<RwLock<ChatStreamState>>,
    pub(super) changed: Arc<Notify>,
}

impl std::fmt::Debug for ChatStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatStream")
            .field("job_id", &self.job_id)
            .field("complete", &self.state.read().complete)
            .finish_non_exhaustive()
    }
}

impl ChatStream {
    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    #[must_use]
    pub fn content(&self) -> String {
        self.state.read().content.clone()
    }

    #[must_use]
    pub fn final_result(&self) -> Option<ChatCompletion> {
        completion_from_state(&self.job_id, &self.state.read())
    }

    pub async fn wait(&self, timeout: Option<Duration>) -> Result<ChatCompletion> {
        let wait = async {
            loop {
                let changed = self.changed.notified();
                {
                    let state = self.state.read();
                    if let Some(error) = &state.error {
                        return Err(error.clone().into());
                    }
                    if let Some(completion) = completion_from_state(&self.job_id, &state) {
                        return Ok(completion);
                    }
                }
                changed.await;
            }
        };
        if let Some(timeout) = timeout {
            tokio::time::timeout(timeout, wait)
                .await
                .map_err(|_| Error::Timeout(format!("chat completion {} timed out", self.job_id)))?
        } else {
            wait.await
        }
    }
}

impl Stream for ChatStream {
    type Item = Result<ChatChunk>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

pub(super) fn completion_from_state(
    job_id: &str,
    state: &ChatStreamState,
) -> Option<ChatCompletion> {
    if !state.complete || state.error.is_some() {
        return None;
    }
    let tool_calls = state
        .tool_calls
        .values()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect();
    Some(ChatCompletion {
        job_id: job_id.to_owned(),
        content: state.content.clone(),
        role: if state.role.is_empty() {
            "assistant".into()
        } else {
            state.role.clone()
        },
        finish_reason: state.finish_reason.clone().unwrap_or_else(|| "stop".into()),
        usage: state.usage.clone().unwrap_or_else(
            || json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}),
        ),
        time_taken: state.time_taken,
        worker_name: state.worker_name.clone(),
        cost: state.cost.clone(),
        tool_calls,
        tool_history: None,
    })
}

#[cfg(test)]
mod tests;
