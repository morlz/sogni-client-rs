use super::*;
use crate::projects::project::ProjectInner;
mod enhancement;
mod runtime;

#[derive(Clone)]
pub struct Job {
    inner: Arc<JobInner>,
}

struct JobInner {
    session: crate::auth::RequestSession,
    result_evidence: RwLock<Option<api::ResultMediaEvidence>>,
    state: RwLock<JobSnapshot>,
    events: EventBus,
    client: Arc<ApiClient>,
    project: Weak<ProjectInner>,
    enhancement_project: RwLock<Option<Project>>,
    project_media_type: String,
    output_format: Option<String>,
    runtime: parking_lot::Mutex<runtime::ProcessingRuntime>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("state", &self.inner.state.read())
            .finish()
    }
}

impl Job {
    pub(super) fn new(
        state: JobSnapshot,
        client: Arc<ApiClient>,
        project: Weak<ProjectInner>,
        project_media_type: String,
        output_format: Option<String>,
        params: &Value,
    ) -> Self {
        let session = project
            .upgrade()
            .map(|project| project.session.clone())
            .unwrap_or_else(|| client.rest.request_session());
        Self {
            inner: Arc::new(JobInner {
                session,
                result_evidence: RwLock::new(None),
                state: RwLock::new(state),
                events: EventBus::default(),
                client,
                project,
                enhancement_project: RwLock::new(None),
                project_media_type,
                output_format,
                runtime: parking_lot::Mutex::new(runtime::ProcessingRuntime::new(params)),
            }),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> JobSnapshot {
        self.inner.state.read().clone()
    }

    #[must_use]
    pub fn id(&self) -> String {
        self.inner.state.read().id.clone()
    }

    #[must_use]
    pub fn status(&self) -> JobStatus {
        self.inner.state.read().status
    }

    /// Current server explanation while this result remains pending.
    #[must_use]
    pub fn waiting_reason(&self) -> Option<WaitingReason> {
        self.inner.state.read().waiting_reason.clone()
    }

    #[must_use]
    pub fn progress(&self) -> u8 {
        job_progress(&self.inner.state.read())
    }

    #[must_use]
    pub fn result_url(&self) -> Option<String> {
        self.inner.state.read().result_url.clone()
    }

    /// Logical position retained when a worker attempt receives a new job id.
    #[must_use]
    pub fn job_index(&self) -> Option<u64> {
        self.inner
            .state
            .read()
            .extra
            .get("jobIndex")
            .and_then(Value::as_u64)
    }

    #[must_use]
    pub fn last_frame_url(&self) -> Option<String> {
        self.inner.state.read().last_frame_url.clone()
    }

    /// Refresh the signed URL of a requested final-frame export.
    pub async fn get_last_frame_url(&self) -> Result<String> {
        self.inner.session.check()?;
        let state = self.snapshot();
        let query = json!({
            "jobId":state.project_id, "id":state.id, "type":"complete", "artifact":"lastFrame"
        });
        let response = self
            .inner
            .session
            .run(
                self.inner
                    .client
                    .rest
                    .get("/v1/media/downloadUrl", Some(&query)),
            )
            .await?;
        let url = response
            .pointer("/data/downloadUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Protocol("download URL response missing data.downloadUrl".into())
            })?
            .to_owned();
        self.update(
            |state| state.last_frame_url = Some(url.clone()),
            &["lastFrameUrl"],
        );
        Ok(url)
    }

    /// Media produced by the model, which may differ from the request type.
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.inner.project_media_type
    }

    #[must_use]
    pub fn provenance(&self) -> Option<JobProvenance> {
        self.inner.state.read().provenance.clone()
    }

    #[must_use]
    pub fn preparation(&self) -> Option<JobPreparation> {
        self.inner.state.read().preparation()
    }

    #[must_use]
    pub fn is_withheld(&self) -> bool {
        let state = self.inner.state.read();
        state.is_nsfw && !state.nsfw_detected
    }

    #[must_use]
    pub fn has_result_media(&self) -> bool {
        self.status() == JobStatus::Completed && !self.is_withheld()
    }

    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.inner.events.subscribe()
    }

    pub(super) fn request_session(&self) -> crate::auth::RequestSession {
        self.inner.session.clone()
    }

    pub(super) fn record_result_evidence(&self, data: &Value) {
        if let Some(evidence) = api::result_media_evidence(data) {
            *self.inner.result_evidence.write() = Some(evidence);
        }
    }

    pub async fn get_result_url(&self) -> Result<String> {
        self.inner.session.check()?;
        if let Some(url) = self.result_url() {
            return Ok(url);
        }
        let state = self.snapshot();
        if state.status != JobStatus::Completed {
            return Err(Error::InvalidInput("job is not completed yet".into()));
        }
        if self.is_withheld() {
            return Err(Error::InvalidInput("job result was withheld".into()));
        }
        let parent = self.inner.project.upgrade().ok_or(Error::Closed)?;
        let api = ProjectsApi {
            inner: parent.api.upgrade().ok_or(Error::Closed)?,
        };
        let params = parent.state.read().params.clone();
        let evidence =
            self.inner.result_evidence.read().clone().or_else(|| {
                api::result_media_evidence(&json!({"outputFormat":state.output_format}))
            });
        let kind = api
            .result_media_kind(
                params.get("modelId").and_then(Value::as_str),
                Some(self.media_type()),
                evidence.as_ref(),
            )
            .ok_or_else(|| Error::Protocol("result media kind is unknown".into()))?;
        let content_type = match (
            kind.as_str(),
            state
                .output_format
                .as_deref()
                .or(self.inner.output_format.as_deref()),
        ) {
            ("audio", Some("flac")) => Some("audio/flac"),
            ("audio", Some("wav")) => Some("audio/wav"),
            ("audio", _) => Some("audio/mpeg"),
            ("model", _) => Some("model/gltf-binary"),
            ("video", Some("webm")) => Some("video/webm"),
            ("image", Some("jpg" | "jpeg")) => Some("image/jpeg"),
            ("image", Some("webp")) => Some("image/webp"),
            ("image", Some("png")) => Some("image/png"),
            _ => None,
        };
        let content_type = content_type.or_else(|| {
            evidence
                .as_ref()
                .filter(|evidence| {
                    evidence.kind == kind
                        && matches!(kind, ResultMediaKind::Audio | ResultMediaKind::Image)
                })
                .and_then(|evidence| evidence.content_type.as_deref())
        });
        let url = self
            .inner
            .session
            .run(api.mint_result_url(&state.project_id, &state.id, kind, content_type))
            .await?;
        self.update(|state| state.result_url = Some(url.clone()), &["resultUrl"]);
        Ok(url)
    }

    pub async fn get_result_data(&self) -> Result<Bytes> {
        self.inner.session.check()?;
        let url = Url::parse(&self.get_result_url().await?)?;
        self.inner
            .session
            .run(self.inner.client.rest.get_bytes(url))
            .await
    }

    /// The currently running or most recent image-enhancement project.
    #[must_use]
    pub fn enhancement_project(&self) -> Option<Project> {
        self.inner.enhancement_project.read().clone()
    }

    /// Enhance a completed image job using the same defaults as the TypeScript client.
    ///
    /// `strength` accepts `"light"`, `"medium"`, or `"heavy"`; unknown values
    /// retain the upstream medium-strength behavior. `overrides` may contain
    /// `positivePrompt`, `stylePrompt`, and `tokenType`.
    pub async fn enhance(
        &self,
        strength: &str,
        overrides: Option<&Value>,
    ) -> Result<Option<String>> {
        let session = self.request_session();
        session
            .run(self.enhance_in_session(strength, overrides))
            .await
    }

    async fn enhance_in_session(
        &self,
        strength: &str,
        overrides: Option<&Value>,
    ) -> Result<Option<String>> {
        let parent = self.inner.project.upgrade().ok_or(Error::Closed)?;
        let parent_params = parent.state.read().params.clone();
        if parent_params.get("type").and_then(Value::as_str) != Some("image")
            || self.media_type() != "image"
        {
            return Err(Error::InvalidInput(
                "enhancement is only available for images".into(),
            ));
        }
        if parent_params
            .get("modelId")
            .and_then(Value::as_str)
            .is_some_and(crate::projects::is_segmentation_model)
        {
            return Err(Error::InvalidInput(
                "Enhancement is not available for segmentation masks".into(),
            ));
        }
        if self.status() != JobStatus::Completed {
            return Err(Error::InvalidInput("job is not completed yet".into()));
        }
        if self.is_withheld() {
            return Err(Error::InvalidInput(
                "job did not pass the NSFW filter".into(),
            ));
        }
        let overrides = overrides.and_then(Value::as_object);
        let inherited_string = |name: &str| {
            overrides
                .and_then(|values| values.get(name))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .or_else(|| parent_params.get(name).and_then(Value::as_str))
                .unwrap_or_default()
                .to_owned()
        };
        let api = ProjectsApi {
            inner: parent.api.upgrade().ok_or(Error::Closed)?,
        };
        let size = self.enhancement_size(&parent_params, &api).await?;
        let data = self.get_result_data().await?;
        let format = self.inner.output_format.as_deref().unwrap_or("png");
        let content_type = match format {
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        };
        let mut request =
            ProjectRequest::image("krea2_turbo_fp8_scaled", inherited_string("positivePrompt"))
                .network(Network::Fast)
                .steps(8)
                .guidance(1.0)
                .number_of_media(1)
                .param("numberOfPreviews", 0)
                .param("negativePrompt", "")
                .param("stylePrompt", inherited_string("stylePrompt"))
                .param(
                    "startingImageStrength",
                    1.0 - enhancement_strength(strength),
                )
                .asset(
                    AssetRole::StartingImage,
                    MediaSource::named_bytes(data, format!("source.{format}"), content_type),
                );
        if let Some(value) = overrides
            .and_then(|values| values.get("tokenType"))
            .filter(|value| !value.is_null())
            .or_else(|| {
                parent_params
                    .get("tokenType")
                    .filter(|value| !value.is_null())
            })
        {
            request = request.param("tokenType", value.clone());
        }
        let job_seed = self.snapshot().seed.map(|seed| json!(seed));
        if let Some(value) = job_seed
            .as_ref()
            .or_else(|| parent_params.get("seed").filter(|value| !value.is_null()))
        {
            request = request.param("seed", value.clone());
        }
        if let Some((width, height)) = size {
            request = request
                .param("sizePreset", "custom")
                .param("width", width)
                .param("height", height);
        }
        let project = api.create(request).await?;
        *self.inner.enhancement_project.write() = Some(project.clone());
        self.inner
            .events
            .emit("updated", json!(["enhancementProject"]));
        let results = project.wait_for_completion(None).await;
        self.inner
            .events
            .emit("updated", json!(["enhancementProject"]));
        Ok(results?.into_iter().next())
    }

    pub(super) fn update(&self, apply: impl FnOnce(&mut JobSnapshot), keys: &[&str]) {
        let network = self
            .inner
            .project
            .upgrade()
            .and_then(|project| project.api.upgrade())
            .and_then(|api| *api.runtime_network.read());
        {
            let mut state = self.inner.state.write();
            apply(&mut state);
            if state.status != JobStatus::Pending {
                state.waiting_reason = None;
            }
            if !keys.is_empty() && keys.iter().all(|key| *key == "waitingReason") {
                drop(state);
                self.inner.events.emit("updated", json!(keys));
                return;
            }
            self.inner.runtime.lock().observe(
                &state,
                keys.contains(&"status"),
                network,
                tokio::time::Instant::now(),
            );
        }
        self.inner.events.emit("updated", json!(keys));
    }

    pub(crate) fn processing_deadline(&self) -> Option<tokio::time::Instant> {
        let state = self.inner.state.read();
        self.inner.runtime.lock().deadline(&state)
    }

    pub(super) fn suspend_processing_deadline(&self) {
        self.inner.runtime.lock().suspend();
    }
}
