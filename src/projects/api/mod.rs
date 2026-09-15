use super::*;
mod costs;
mod create;
mod loras;
mod media;
mod models;
mod recovery;
mod reusable;
mod status;
mod submission_recovery;
pub use reusable::{ReusableUploads, SavedUpload, SavedUploadBinding};

#[derive(Clone)]
pub struct ProjectsApi {
    pub(super) inner: Arc<ProjectsInner>,
}

pub(super) struct TimedValue {
    pub(super) value: Value,
    pub(super) loaded_at: Instant,
}

pub(super) struct ProjectsInner {
    pub(super) client: Arc<ApiClient>,
    pub(super) projects: RwLock<HashMap<String, Project>>,
    pub(super) available_models: RwLock<Vec<Value>>,
    // Budget unpinned jobs against the last server announcement, not the
    // requested client network; an unknown network uses the relaxed floor.
    pub(super) runtime_network: RwLock<Option<Network>>,
    pub(super) supported_models: RwLock<Option<TimedValue>>,
    pub(super) model_tiers: RwLock<Option<TimedValue>>,
    pub(super) events: EventBus,
    pub(super) recovered_completed_ids: RwLock<HashSet<String>>,
    pub(super) sync_lock: Mutex<()>,
    pub(super) assets: ReusableUploads,
    pub(super) submission: parking_lot::Mutex<submission_recovery::SubmissionRecovery>,
}

impl std::fmt::Debug for ProjectsApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectsApi")
            .field("tracked_projects", &self.inner.projects.read().len())
            .finish_non_exhaustive()
    }
}

impl ProjectsApi {
    pub(crate) fn new(client: Arc<ApiClient>) -> Self {
        let inner = Arc::new(ProjectsInner {
            assets: ReusableUploads::new(client.rest.clone()),
            submission: parking_lot::Mutex::new(submission_recovery::SubmissionRecovery::default()),
            client,
            projects: RwLock::new(HashMap::new()),
            available_models: RwLock::new(Vec::new()),
            runtime_network: RwLock::new(None),
            supported_models: RwLock::new(None),
            model_tiers: RwLock::new(None),
            events: EventBus::default(),
            recovered_completed_ids: RwLock::new(HashSet::new()),
            sync_lock: Mutex::new(()),
        });
        listen_for_project_events(&inner);
        Self { inner }
    }

    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.inner.events.subscribe()
    }

    #[must_use]
    pub fn assets(&self) -> ReusableUploads {
        self.inner.assets.clone()
    }

    #[must_use]
    pub fn tracked_projects(&self) -> Vec<Project> {
        self.inner.projects.read().values().cloned().collect()
    }

    #[must_use]
    pub fn available_models(&self) -> Vec<Value> {
        self.inner.available_models.read().clone()
    }

    #[must_use]
    pub fn is_video_model_id(&self, model_id: &str) -> bool {
        cached_model_media(&self.inner.supported_models, model_id)
            .map_or_else(|| is_video_model(model_id), |media| media == "video")
    }

    #[must_use]
    pub fn is_audio_model_id(&self, model_id: &str) -> bool {
        cached_model_media(&self.inner.supported_models, model_id)
            .map_or_else(|| is_audio_model(model_id), |media| media == "audio")
    }

    /// Consult advertised media metadata, falling back to known 3D model IDs.
    #[must_use]
    pub fn is_model_artifact_model_id(&self, model_id: &str) -> bool {
        is_model_artifact_model(model_id)
            || cached_model_media(&self.inner.supported_models, model_id)
                .is_some_and(|media| media == "model")
    }

    pub async fn wait_for_models(&self, timeout: Duration) -> Result<Vec<Value>> {
        if !self.available_models().is_empty() {
            return Ok(self.available_models());
        }
        let mut events = self.subscribe();
        tokio::time::timeout(timeout, async {
            loop {
                let event = events.recv().await.map_err(|error| {
                    Error::Transport(format!("model event stream closed: {error}"))
                })?;
                if event.name == "availableModels" {
                    let models = self.available_models();
                    if !models.is_empty() {
                        return Ok(models);
                    }
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout("waiting for available models".into()))?
    }
}
