//! Private subscriber uploads. Eligibility and asset verification remain server-owned.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore, watch};

use crate::{ApiError, Error, MediaSource, Result, transport::RestClient, utils::path_segment};

const MAX_FILE_BYTES: usize = 100 * 1024 * 1024;
const SUPPORTED_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "video/mp4",
    "video/quicktime",
    "video/webm",
    "audio/mp4",
    "audio/mpeg",
    "audio/flac",
    "audio/wav",
    "audio/x-wav",
    "audio/wave",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedUpload {
    pub id: String,
    pub name: String,
    pub bytes: u64,
    pub content_type: String,
    pub state: String,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedUploadBinding {
    pub project_id: String,
    #[serde(rename = "type")]
    pub asset_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

struct Availability {
    session: watch::Receiver<bool>,
    capability: Option<(Instant, bool)>,
    blocked_until: Option<Instant>,
}

struct ReusableInner {
    rest: RestClient,
    lanes: Semaphore,
    availability: Mutex<Availability>,
}

/// Upload media once, then bind verified private assets to later projects.
#[derive(Clone)]
pub struct ReusableUploads {
    inner: Arc<ReusableInner>,
}

impl std::fmt::Debug for ReusableUploads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReusableUploads").finish_non_exhaustive()
    }
}

impl ReusableUploads {
    pub(super) fn new(rest: RestClient) -> Self {
        Self {
            inner: Arc::new(ReusableInner {
                availability: Mutex::new(Availability {
                    session: rest.auth_updates(),
                    capability: None,
                    blocked_until: None,
                }),
                rest,
                lanes: Semaphore::new(2),
            }),
        }
    }

    /// List private assets and the current server-advertised library limits.
    pub async fn list(&self) -> Result<Value> {
        data(self.inner.rest.get("/v1/assets", None).await?)
    }

    pub async fn remove(&self, id: &str) -> Result<()> {
        require_id(id)?;
        self.inner
            .rest
            .delete(&format!("/v1/assets/{}", path_segment(id)))
            .await?;
        self.inner.availability.lock().await.blocked_until = None;
        Ok(())
    }

    pub async fn bind(&self, id: &str, binding: &SavedUploadBinding) -> Result<()> {
        self.bind_in_session(id, binding, &self.inner.rest.auth_updates())
            .await
    }

    async fn bind_in_session(
        &self,
        id: &str,
        binding: &SavedUploadBinding,
        session: &watch::Receiver<bool>,
    ) -> Result<()> {
        require_id(id)?;
        self.post_busy(
            &format!("/v1/assets/{}/bind", path_segment(id)),
            &serde_json::to_value(binding)?,
            session,
        )
        .await?;
        Ok(())
    }

    /// Explicitly save a file; the server verifies it before acknowledging readiness.
    pub async fn upload(&self, source: &MediaSource) -> Result<SavedUpload> {
        let session = self.inner.rest.auth_updates();
        let _lane = self
            .inner
            .lanes
            .acquire()
            .await
            .map_err(|_| Error::Closed)?;
        assert_session(&session)?;
        let media = source.read().await?;
        assert_session(&session)?;
        let content_type = media
            .content_type
            .as_deref()
            .ok_or_else(|| Error::InvalidInput("saved uploads require a content type".into()))?;
        let prepared = self
            .prepare(&media.data, content_type, &media.file_name, &session)
            .await?;
        self.finish(prepared, media.data, &session).await
    }

    /// Try the reusable upload path. A refusal before preparation may use the
    /// ordinary project upload; transfer, verification and binding errors surface.
    pub(super) async fn try_bind(
        &self,
        bytes: Bytes,
        content_type: Option<&str>,
        name: &str,
        binding: &SavedUploadBinding,
    ) -> Result<bool> {
        let Some(content_type) = content_type.filter(|kind| SUPPORTED_TYPES.contains(kind)) else {
            return Ok(false);
        };
        if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
            return Ok(false);
        }
        let session = self.inner.rest.auth_updates();
        if !self.can_save(&session).await? {
            return Ok(false);
        }
        let _lane = self
            .inner
            .lanes
            .acquire()
            .await
            .map_err(|_| Error::Closed)?;
        assert_session(&session)?;
        if self.blocked().await {
            return Ok(false);
        }
        let prepared = match self.prepare(&bytes, content_type, name, &session).await {
            Ok(prepared) => prepared,
            Err(Error::Api(error)) if matches!(error.status, 400 | 403 | 404 | 409 | 410 | 503) => {
                assert_session(&session)?;
                if error.status == 409
                    && error
                        .message
                        .starts_with("Your saved upload library is full.")
                {
                    self.inner.availability.lock().await.blocked_until =
                        Some(Instant::now() + Duration::from_secs(15 * 60));
                }
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let saved = self.finish(prepared, bytes, &session).await?;
        self.bind_in_session(&saved.id, binding, &session).await?;
        Ok(true)
    }

    async fn prepare(
        &self,
        bytes: &Bytes,
        content_type: &str,
        name: &str,
        session: &watch::Receiver<bool>,
    ) -> Result<Value> {
        if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
            return Err(ApiError::new(
                400,
                json!({"status":"error", "errorCode":0,
                "message":"Choose a saved upload no larger than 100 MiB."}),
            )
            .into());
        }
        assert_session(session)?;
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        let result = self
            .post_busy(
                "/v1/assets/prepare",
                &json!({
                    "sha256": sha256, "bytes": bytes.len(), "contentType":content_type, "name":name,
                }),
                session,
            )
            .await?;
        data(result)
    }

    async fn finish(
        &self,
        prepared: Value,
        bytes: Bytes,
        session: &watch::Receiver<bool>,
    ) -> Result<SavedUpload> {
        assert_session(session)?;
        if prepared.get("state").and_then(Value::as_str) == Some("ready") {
            return Ok(serde_json::from_value(prepared)?);
        }
        let url = prepared
            .get("uploadUrl")
            .and_then(Value::as_str)
            .ok_or_else(unprepared)?
            .parse()?;
        let fields = prepared
            .get("uploadHeaders")
            .and_then(Value::as_object)
            .ok_or_else(unprepared)?;
        let mut headers = HeaderMap::new();
        for (name, value) in fields {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| unprepared())?;
            let value = HeaderValue::from_str(value.as_str().ok_or_else(unprepared)?)
                .map_err(|_| unprepared())?;
            headers.insert(name, value);
        }
        self.inner.rest.put_saved_asset(url, bytes, headers).await?;
        assert_session(session)?;
        let id = prepared
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(unprepared)?;
        let finalized = self
            .post_busy(
                &format!("/v1/assets/{}/finalize", path_segment(id)),
                &json!({}),
                session,
            )
            .await?;
        Ok(serde_json::from_value(data(finalized)?)?)
    }

    async fn post_busy(
        &self,
        path: &str,
        body: &Value,
        session: &watch::Receiver<bool>,
    ) -> Result<Value> {
        for attempt in 0..=4 {
            assert_session(session)?;
            let result = self.inner.rest.post(path, body).await;
            assert_session(session)?;
            match result {
                Err(Error::Api(error)) if error.status == 423 && attempt < 4 => {
                    tokio::time::sleep(Duration::from_millis(250 * (1 << attempt))).await;
                }
                result => return result,
            }
        }
        unreachable!("bounded busy retry always returns")
    }

    async fn blocked(&self) -> bool {
        let mut state = self.inner.availability.lock().await;
        refresh_session(&mut state);
        state
            .blocked_until
            .is_some_and(|until| until > Instant::now())
    }

    async fn can_save(&self, session: &watch::Receiver<bool>) -> Result<bool> {
        let mut state = self.inner.availability.lock().await;
        refresh_session(&mut state);
        assert_session(session)?;
        let now = Instant::now();
        if state.blocked_until.is_some_and(|until| until > now) {
            return Ok(false);
        }
        if let Some((until, enabled)) = state.capability.filter(|(until, _)| *until > now) {
            let _ = until;
            return Ok(enabled);
        }
        let result = self.inner.rest.get("/v1/assets/capabilities", None).await;
        assert_session(session)?;
        let enabled = match result {
            Ok(result) => result.pointer("/data/enabled").and_then(Value::as_bool) == Some(true),
            Err(Error::Api(error)) if matches!(error.status, 403 | 404 | 503) => false,
            Err(error) => return Err(error),
        };
        state.capability = Some((Instant::now() + Duration::from_secs(60), enabled));
        Ok(enabled)
    }
}

fn assert_session(session: &watch::Receiver<bool>) -> Result<()> {
    if session.has_changed().unwrap_or(true) {
        Err(Error::InvalidInput(
            "The account changed. Select the upload again.".into(),
        ))
    } else {
        Ok(())
    }
}

fn refresh_session(state: &mut Availability) {
    if state.session.has_changed().unwrap_or(true) {
        state.session.borrow_and_update();
        state.capability = None;
        state.blocked_until = None;
    }
}

fn require_id(id: &str) -> Result<()> {
    if id.trim().is_empty() {
        Err(Error::InvalidInput("saved upload id is required".into()))
    } else {
        Ok(())
    }
}

fn data(response: Value) -> Result<Value> {
    response
        .get("data")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(unprepared)
}

fn unprepared() -> Error {
    Error::Protocol("The saved upload could not be prepared.".into())
}

#[cfg(test)]
mod tests;
