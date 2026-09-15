use std::{collections::HashMap, sync::Arc, time::Duration};

use parking_lot::RwLock;
use reqwest::header::HeaderMap;
use serde_json::Value;

use super::{
    events::listen_for_chat_events, tools::HostedTools, types::ActiveChat,
    validation::parse_attribution,
};
use crate::{Error, EventReceiver, ProjectsApi, Result, event::EventBus, transport::ApiClient};

pub(super) const CHAT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub struct ChatApi {
    pub(super) inner: Arc<ChatInner>,
    pub tools: HostedTools,
}

pub(super) struct ChatInner {
    pub(super) client: Arc<ApiClient>,
    pub(super) projects: ProjectsApi,
    pub(super) active: RwLock<HashMap<String, ActiveChat>>,
    pub(super) models: RwLock<HashMap<String, Value>>,
    pub(super) events: EventBus,
    pub(super) recovery: parking_lot::Mutex<super::events::TransportRecovery>,
}

impl std::fmt::Debug for ChatApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatApi")
            .field("active_streams", &self.inner.active.read().len())
            .field("models", &self.inner.models.read().len())
            .finish_non_exhaustive()
    }
}

impl ChatApi {
    pub(crate) fn new(client: Arc<ApiClient>, projects: ProjectsApi) -> Self {
        let inner = Arc::new(ChatInner {
            client,
            projects,
            active: RwLock::new(HashMap::new()),
            models: RwLock::new(HashMap::new()),
            events: EventBus::default(),
            recovery: parking_lot::Mutex::new(super::events::TransportRecovery::default()),
        });
        listen_for_chat_events(&inner);
        Self {
            inner,
            tools: HostedTools,
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.inner.events.subscribe()
    }

    #[must_use]
    pub fn models(&self) -> HashMap<String, Value> {
        self.inner.models.read().clone()
    }

    pub async fn wait_for_models(&self, timeout: Duration) -> Result<HashMap<String, Value>> {
        if !self.models().is_empty() {
            return Ok(self.models());
        }
        let mut events = self.subscribe();
        tokio::time::timeout(timeout, async {
            loop {
                let event = events.recv().await.map_err(|error| {
                    Error::Transport(format!("chat model event stream closed: {error}"))
                })?;
                if event.name == "modelsUpdated" && !self.models().is_empty() {
                    return Ok(self.models());
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout("waiting for LLM models".into()))?
    }

    pub(super) fn attribution_headers(
        &self,
        params: &Value,
        app_source: Option<&str>,
        operation_id: &str,
    ) -> Result<HeaderMap> {
        let attribution = parse_attribution(params.get("attribution"))?;
        let workload = self
            .inner
            .client
            .resolve_workload_attribution(attribution.as_ref(), Some(operation_id));
        self.inner
            .client
            .attribution_headers(app_source, workload.as_ref())
    }
}
