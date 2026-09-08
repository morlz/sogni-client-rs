use super::*;
use crate::projects::project::ProjectInner;

#[derive(Clone)]
pub struct Job {
    inner: Arc<JobInner>,
}

struct JobInner {
    state: RwLock<JobSnapshot>,
    events: EventBus,
    client: Arc<ApiClient>,
    project: Weak<ProjectInner>,
    enhancement_project: RwLock<Option<Project>>,
    project_media_type: String,
    output_format: Option<String>,
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
    ) -> Self {
        Self {
            inner: Arc::new(JobInner {
                state: RwLock::new(state),
                events: EventBus::default(),
                client,
                project,
                enhancement_project: RwLock::new(None),
                project_media_type,
                output_format,
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

    #[must_use]
    pub fn progress(&self) -> u8 {
        job_progress(&self.inner.state.read())
    }

    #[must_use]
    pub fn result_url(&self) -> Option<String> {
        self.inner.state.read().result_url.clone()
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

    pub async fn get_result_url(&self) -> Result<String> {
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
        let content_type = match (
            self.inner.project_media_type.as_str(),
            self.inner.output_format.as_deref(),
        ) {
            ("audio", Some("flac")) => Some("audio/flac"),
            ("audio", Some("wav")) => Some("audio/wav"),
            ("audio", _) => Some("audio/mpeg"),
            ("model", _) => Some("model/gltf-binary"),
            ("image", Some("jpg" | "jpeg")) => Some("image/jpeg"),
            ("image", Some("webp")) => Some("image/webp"),
            ("image", Some("png")) => Some("image/png"),
            _ => None,
        };
        let query = if matches!(self.media_type(), "video" | "audio" | "model") {
            json!({
                "jobId": state.project_id,
                "id": state.id,
                "type": "complete",
                "contentType": content_type,
            })
        } else {
            json!({
                "jobId": state.project_id,
                "imageId": state.id,
                "type": "complete",
                "contentType": content_type,
            })
        };
        let endpoint = if matches!(self.media_type(), "video" | "audio" | "model") {
            "/v1/media/downloadUrl"
        } else {
            "/v1/image/downloadUrl"
        };
        let response = self.inner.client.rest.get(endpoint, Some(&query)).await?;
        let url = response
            .pointer("/data/downloadUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Protocol("download URL response missing data.downloadUrl".into())
            })?
            .to_owned();
        self.update(|state| state.result_url = Some(url.clone()), &["resultUrl"]);
        Ok(url)
    }

    pub async fn get_result_data(&self) -> Result<Bytes> {
        let url = Url::parse(&self.get_result_url().await?)?;
        self.inner.client.rest.get_bytes(url).await
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
        let parent = self.inner.project.upgrade().ok_or(Error::Closed)?;
        let parent_params = parent.state.read().params.clone();
        if parent_params.get("type").and_then(Value::as_str) != Some("image")
            || self.media_type() != "image"
        {
            return Err(Error::InvalidInput(
                "enhancement is only available for images".into(),
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
        let data = self.get_result_data().await?;
        let format = self.inner.output_format.as_deref().unwrap_or("png");
        let content_type = match format {
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        };
        let mut request =
            ProjectRequest::image("flux1-schnell-fp8", inherited_string("positivePrompt"))
                .network(Network::Fast)
                .steps(5)
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
        if let Some(value) = parent_params
            .get("sizePreset")
            .filter(|value| !value.is_null())
        {
            request = request.param("sizePreset", value.clone());
        }
        for dimension in ["width", "height"] {
            if let Some(value) = parent_params
                .get(dimension)
                .filter(|value| !value.is_null())
            {
                request = request.param(dimension, value.clone());
            }
        }
        let api = parent.api.upgrade().ok_or(Error::Closed)?;
        let project = ProjectsApi { inner: api }.create(request).await?;
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
        apply(&mut self.inner.state.write());
        self.inner.events.emit("updated", json!(keys));
    }
}
