use super::*;

/// The media produced by a result, used to select its signed-download endpoint.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResultMediaKind {
    Image,
    Video,
    Audio,
    Model,
}

impl ResultMediaKind {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            "model" => Some(Self::Model),
            _ => None,
        }
    }
    pub(in crate::projects) const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Model => "model",
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::projects) struct ResultMediaEvidence {
    pub kind: ResultMediaKind,
    pub content_type: Option<String>,
}

pub(in crate::projects) fn result_media_evidence(raw: &Value) -> Option<ResultMediaEvidence> {
    // Live frames carry these fields directly; stored results may nest them.
    // Null means absent, while a non-null malformed value remains non-evidence.
    let artifacts = raw
        .get("artifacts")
        .filter(|value| !value.is_null())
        .or_else(|| raw.pointer("/result/artifacts"));
    let mut found = Vec::new();
    for artifact in artifacts.and_then(Value::as_array).into_iter().flatten() {
        if artifact.get("success").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let Some(content_type) = artifact.get("contentType").and_then(Value::as_str) else {
            continue;
        };
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let kind = if mime.starts_with("model/") {
            ResultMediaKind::Model
        } else if mime.starts_with("video/") {
            ResultMediaKind::Video
        } else if mime.starts_with("audio/") {
            ResultMediaKind::Audio
        } else if mime.starts_with("image/") {
            ResultMediaKind::Image
        } else {
            continue;
        };
        found.push(ResultMediaEvidence {
            kind,
            content_type: Some(content_type.trim().into()),
        });
    }
    for kind in [
        ResultMediaKind::Model,
        ResultMediaKind::Video,
        ResultMediaKind::Audio,
        ResultMediaKind::Image,
    ] {
        if let Some(value) = found.iter().find(|value| value.kind == kind) {
            return Some(value.clone());
        }
    }
    let format = raw
        .get("outputFormat")
        .filter(|value| !value.is_null())
        .or_else(|| raw.pointer("/result/outputFormat"))
        .and_then(Value::as_str)?
        .trim()
        .to_ascii_lowercase();
    let (kind, content_type) = match format.as_str() {
        "mp4" | "mov" => (ResultMediaKind::Video, None),
        "mp3" => (ResultMediaKind::Audio, Some("audio/mpeg")),
        "wav" => (ResultMediaKind::Audio, Some("audio/wav")),
        "flac" => (ResultMediaKind::Audio, Some("audio/flac")),
        "glb" => (ResultMediaKind::Model, Some("model/gltf-binary")),
        "png" | "jpg" | "jpeg" | "webp" => (ResultMediaKind::Image, None),
        _ => return None,
    };
    Some(ResultMediaEvidence {
        kind,
        content_type: content_type.map(str::to_owned),
    })
}

impl ProjectsApi {
    pub(in crate::projects) fn result_media_kind(
        &self,
        model_id: Option<&str>,
        project_type: Option<&str>,
        evidence: Option<&ResultMediaEvidence>,
    ) -> Option<ResultMediaKind> {
        if let Some(model_id) = model_id {
            if is_model_artifact_model(model_id) {
                return Some(ResultMediaKind::Model);
            }
            if let Some(kind) = cached_model_media(&self.inner.supported_models, model_id)
                .and_then(|value| ResultMediaKind::parse(&value))
            {
                return Some(kind);
            }
            if is_video_model(model_id) {
                return Some(ResultMediaKind::Video);
            }
            if is_audio_model(model_id) {
                return Some(ResultMediaKind::Audio);
            }
        }
        evidence
            .map(|evidence| evidence.kind)
            .or_else(|| project_type.and_then(ResultMediaKind::parse))
    }

    pub(in crate::projects) async fn mint_result_url(
        &self,
        project_id: &str,
        job_id: &str,
        kind: ResultMediaKind,
        content_type: Option<&str>,
    ) -> Result<String> {
        let session = self.inner.client.rest.request_session();
        let content_type = content_type.filter(|value| {
            let mime = value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            match kind {
                ResultMediaKind::Image => mime.starts_with("image/"),
                ResultMediaKind::Audio => mime.starts_with("audio/"),
                // Preserve the Rust client's explicit WebM output override.
                // Other video formats use the stored result without a MIME override.
                ResultMediaKind::Video => mime == "video/webm",
                ResultMediaKind::Model => false,
            }
        });
        let key = (
            session.id(),
            project_id.to_uppercase(),
            job_id.to_uppercase(),
        );
        let known_media = self.inner.media_results.lock().contains(&key);
        if kind == ResultMediaKind::Image && !known_media {
            let query = json!({"jobId":project_id,"imageId":job_id,"type":"complete","contentType":content_type});
            match session
                .run(
                    self.inner
                        .client
                        .rest
                        .get("/v1/image/downloadUrl", Some(&query)),
                )
                .await
            {
                Ok(response) => return signed_url(response),
                Err(Error::Api(error))
                    if error.status == 404
                        && error
                            .payload
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or(&error.message)
                            .contains("/v1/media/downloadUrl") =>
                {
                    let mut remembered = self.inner.media_results.lock();
                    remembered.retain(|value| value != &key);
                    remembered.push_back(key);
                    if remembered.len() > 1000 {
                        remembered.pop_front();
                    }
                }
                Err(error) => return Err(error),
            }
        }
        let content_type = match kind {
            ResultMediaKind::Model => Some("model/gltf-binary"),
            ResultMediaKind::Audio | ResultMediaKind::Video => content_type,
            ResultMediaKind::Image => None,
        };
        let query =
            json!({"jobId":project_id,"id":job_id,"type":"complete","contentType":content_type});
        signed_url(
            session
                .run(
                    self.inner
                        .client
                        .rest
                        .get("/v1/media/downloadUrl", Some(&query)),
                )
                .await?,
        )
    }
}

fn signed_url(response: Value) -> Result<String> {
    response
        .pointer("/data/downloadUrl")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Error::Protocol("download URL response missing data.downloadUrl".into()))
}
