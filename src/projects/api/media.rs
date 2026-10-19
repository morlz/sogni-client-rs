use super::*;

#[cfg(test)]
#[path = "media/live_probe.rs"]
mod live_probe;

impl ProjectsApi {
    /// Upload a guide under caller-reserved UUIDs before project submission.
    /// Persist the UUIDs first when the source must be verified or recovered.
    /// Submit afterward with `startingImage=true` and without a duplicate asset.
    /// This uploads bytes only; it does not create a generation project.
    pub async fn upload_guide_image(
        &self,
        project_id: &str,
        image_id: &str,
        source: &MediaSource,
    ) -> Result<()> {
        let session = self.inner.client.rest.request_session();
        session
            .run(async {
                let project_id = super::create::normalize_project_id(project_id)?;
                let image_id = uuid::Uuid::parse_str(image_id)
                    .map_err(|_| Error::InvalidInput("image id must be a UUID".into()))?
                    .to_string()
                    .to_uppercase();
                let media = source.read().await?;
                let query = json!({
                    "imageId": image_id, "jobId": project_id, "type": "startingImage",
                    "contentType": media.content_type,
                });
                let upload = self.upload_url(&query).await?;
                self.inner
                    .client
                    .rest
                    .put_bytes(upload, media.data, media.content_type.as_deref())
                    .await
            })
            .await
    }

    pub async fn upload_url(&self, query: &Value) -> Result<Url> {
        response_url(
            self.inner
                .client
                .rest
                .get("/v1/image/uploadUrl", Some(query))
                .await?,
            "uploadUrl",
        )
    }

    pub async fn download_url(&self, query: &Value) -> Result<Url> {
        response_url(
            self.inner
                .client
                .rest
                .get("/v1/image/downloadUrl", Some(query))
                .await?,
            "downloadUrl",
        )
    }

    pub async fn media_upload_url(&self, query: &Value) -> Result<Url> {
        response_url(
            self.inner
                .client
                .rest
                .get("/v1/media/uploadUrl", Some(query))
                .await?,
            "uploadUrl",
        )
    }

    pub async fn media_download_url(&self, query: &Value) -> Result<Url> {
        response_url(
            self.inner
                .client
                .rest
                .get("/v1/media/downloadUrl", Some(query))
                .await?,
            "downloadUrl",
        )
    }

    /// Request the current v2 multipart image upload form.
    pub async fn image_upload_post(&self, query: &Value) -> Result<PresignedPost> {
        self.presigned_post("/v2/image/uploadUrl", query).await
    }

    /// Request the current v2 multipart media upload form.
    pub async fn media_upload_post(&self, query: &Value) -> Result<PresignedPost> {
        self.presigned_post("/v2/media/uploadUrl", query).await
    }

    pub async fn upload_presigned(
        &self,
        post: &PresignedPost,
        source: &MediaSource,
    ) -> Result<Option<Url>> {
        let media = source.read().await?;
        if post
            .max_size_bytes
            .is_some_and(|limit| media.data.len() as u64 > limit)
        {
            return Err(Error::InvalidInput(format!(
                "media is {} bytes, exceeding the {} byte upload limit",
                media.data.len(),
                post.max_size_bytes.expect("checked Some")
            )));
        }
        self.inner
            .client
            .rest
            .post_multipart(
                post.url.clone(),
                &post.fields,
                media.data,
                &media.file_name,
                media.content_type.as_deref(),
            )
            .await?;
        Ok(post.public_url.clone())
    }

    async fn presigned_post(&self, path: &str, query: &Value) -> Result<PresignedPost> {
        let response = self.inner.client.rest.get(path, Some(query)).await?;
        parse_presigned_post(&response)
    }

    pub(super) async fn process_assets(
        &self,
        session: &crate::auth::RequestSession,
        project_id: &str,
        assets: &[(AssetRole, MediaSource)],
        request: &mut Value,
        annotate_video: bool,
    ) -> Result<()> {
        let model_id = request
            .pointer("/keyFrames/0/modelID")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut seen = std::collections::HashSet::new();
        for (role, source) in assets {
            session.check()?;
            match role {
                AssetRole::ContextImage(slot) if !(1..=16).contains(slot) => {
                    return Err(Error::InvalidInput(
                        "context image slot must be between 1 and 16".into(),
                    ));
                }
                AssetRole::ReferenceAudioSlot(slot) if !(1..=3).contains(slot) => {
                    return Err(Error::InvalidInput(
                        "MiniMax H3 audio-reference slot must be between 1 and 3".into(),
                    ));
                }
                AssetRole::ReferenceVideoSlot(slot) if !(1..=3).contains(slot) => {
                    return Err(Error::InvalidInput(
                        "MiniMax H3 video-reference slot must be between 1 and 3".into(),
                    ));
                }
                _ => {}
            }
            let wire_role = effective_asset_wire_name(role, &model_id);
            if !seen.insert(wire_role.clone()) {
                return Err(Error::InvalidInput(format!(
                    "duplicate project asset role {wire_role}"
                )));
            }
            let media = source.read_for_role(role).await?;
            session.check()?;
            let (resource_type, resource_id) = if role.is_media() {
                ["referenceAudio", "referenceVideo"]
                    .into_iter()
                    .find_map(|kind| {
                        wire_role
                            .strip_prefix(kind)
                            .filter(|slot| {
                                !slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit())
                            })
                            .map(|_| (kind, Some(wire_role.as_str())))
                    })
                    .unwrap_or((&wire_role, None))
            } else {
                (wire_role.as_str(), None)
            };
            let query = if role.is_media() {
                json!({
                    "jobId": project_id,
                    "type": resource_type,
                    "id": resource_id,
                    "contentType": media.content_type,
                })
            } else {
                json!({
                    "imageId": new_id(),
                    "jobId": project_id,
                    "type": wire_role,
                    "contentType": media.content_type,
                })
            };
            let reused = self
                .inner
                .assets
                .try_bind(
                    media.data.clone(),
                    media.content_type.as_deref(),
                    &media.file_name,
                    &SavedUploadBinding {
                        project_id: project_id.to_owned(),
                        asset_type: resource_type.to_owned(),
                        id: resource_id.map(str::to_owned),
                    },
                )
                .await?;
            session.check()?;
            if !reused {
                let upload = if role.is_media() {
                    self.media_upload_url(&query).await?
                } else {
                    self.upload_url(&query).await?
                };
                session.check()?;
                self.inner
                    .client
                    .rest
                    .put_bytes(upload, media.data, media.content_type.as_deref())
                    .await?;
                session.check()?;
            }
            // Image uploads advertise their MIME in the resource registration
            // and PUT only. Video and voice-clone audio inputs annotate the wire.
            if let (Some(keyframe), Some(content_type)) = (
                request
                    .pointer_mut("/keyFrames/0")
                    .filter(|_| annotate_video && !matches!(role, AssetRole::KeyframeImage(_))),
                media.content_type,
            ) {
                if matches!(role, AssetRole::ReferenceAudioIdentity) {
                    keyframe["referenceAudioIdentityContentType"] = json!(content_type);
                    if keyframe.get("referenceAudioContentType").is_none() {
                        keyframe["referenceAudioContentType"] = json!(content_type);
                    }
                } else {
                    keyframe[format!("{wire_role}ContentType")] = json!(content_type);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::Bytes,
        extract::{Query, State},
        http::{HeaderMap, Uri},
        routing::{get, put},
    };
    use std::{collections::BTreeMap, time::Duration};
    use tokio::{net::TcpListener, sync::mpsc};

    const WATCHDOG: Duration = Duration::from_secs(10);

    enum Call {
        Registration(String, BTreeMap<String, String>),
        Upload(HeaderMap, Bytes),
    }

    #[derive(Clone)]
    struct Fixture {
        upload: Url,
        calls: mpsc::UnboundedSender<Call>,
    }

    async fn register(
        State(fixture): State<Fixture>,
        uri: Uri,
        Query(query): Query<BTreeMap<String, String>>,
    ) -> Json<Value> {
        fixture
            .calls
            .send(Call::Registration(uri.path().into(), query))
            .unwrap();
        Json(json!({"data":{"uploadUrl":fixture.upload}}))
    }

    #[tokio::test]
    async fn reference_assets_share_content_type_for_registration_upload_and_wire() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint: Url = format!("http://{}/", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let (tx, mut calls) = mpsc::unbounded_channel();
        let router = Router::new()
            .route(
                "/v1/account/me",
                get(|| async { Json(json!({"data":{"username":"fixture"}})) }),
            )
            .route(
                "/v1/assets/capabilities",
                get(|| async { Json(json!({"data":{"enabled":false}})) }),
            )
            .route("/v1/media/uploadUrl", get(register))
            .route("/v1/image/uploadUrl", get(register))
            .route(
                "/upload",
                put(
                    |State(fixture): State<Fixture>, headers: HeaderMap, body: Bytes| async move {
                        fixture.calls.send(Call::Upload(headers, body)).unwrap();
                        Json(json!({}))
                    },
                ),
            )
            .with_state(Fixture {
                upload: endpoint.join("upload").unwrap(),
                calls: tx,
            });
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = crate::SogniClient::builder()
            .disable_socket(true)
            .api_key("local-mime-fixture")
            .rest_endpoint(endpoint)
            .build()
            .await
            .unwrap();

        let flac = b"fLaCfixture-content".to_vec();
        let ogg = b"OggSfixture-content".to_vec();
        let webm = b"\x1a\x45\xdf\xa3fixture-content".to_vec();
        let png = b"\x89PNGfixture-content".to_vec();
        let mut cases = vec![
            (AssetRole::ReferenceAudio, flac.clone(), Some("audio/flac")),
            (AssetRole::ReferenceAudio, ogg.clone(), Some("audio/ogg")),
            (AssetRole::ReferenceAudio, webm.clone(), Some("audio/webm")),
            (AssetRole::ReferenceVideo, ogg.clone(), Some("video/ogg")),
            (AssetRole::ReferenceVideo, webm.clone(), Some("video/webm")),
            (
                AssetRole::ReferenceAudioIdentity,
                webm.clone(),
                Some("audio/webm"),
            ),
            (
                AssetRole::ReferenceAudioIdentity,
                ogg.clone(),
                Some("audio/ogg"),
            ),
            (
                AssetRole::ReferenceAudioSlot(1),
                flac.clone(),
                Some("audio/flac"),
            ),
            (
                AssetRole::ReferenceAudioSlot(2),
                ogg.clone(),
                Some("audio/ogg"),
            ),
            (
                AssetRole::ReferenceAudioSlot(3),
                webm.clone(),
                Some("audio/webm"),
            ),
            (
                AssetRole::ReferenceVideoSlot(1),
                webm.clone(),
                Some("video/webm"),
            ),
            (
                AssetRole::ReferenceVideoSlot(2),
                ogg.clone(),
                Some("video/ogg"),
            ),
            (
                AssetRole::ReferenceVideoSlot(3),
                webm.clone(),
                Some("video/webm"),
            ),
            (AssetRole::KeyframeImage(1), png, Some("image/png")),
            (
                AssetRole::ReferenceAudio,
                b"unknown-reference".to_vec(),
                None,
            ),
        ]
        .drain(..)
        .map(|(role, bytes, expected)| {
            let source = MediaSource::bytes(bytes.clone());
            (role, source, bytes, expected)
        })
        .collect::<Vec<_>>();
        cases.push((
            AssetRole::ReferenceAudio,
            MediaSource::named_bytes(webm.clone(), "guide.webm", "audio/ogg"),
            webm.clone(),
            Some("audio/ogg"),
        ));
        cases.push((
            AssetRole::ReferenceVideo,
            MediaSource::named_bytes(ogg.clone(), "guide.ogg", "video/webm"),
            ogg.clone(),
            Some("video/webm"),
        ));
        let directory = tempfile::tempdir().unwrap();
        for (name, role, bytes, expected) in [
            ("video.mp3", AssetRole::ReferenceVideo, ogg, "video/ogg"),
            ("audio.ogg", AssetRole::ReferenceAudio, webm, "audio/webm"),
            ("audio.mp4", AssetRole::ReferenceAudio, flac, "audio/flac"),
        ] {
            let path = directory.path().join(name);
            tokio::fs::write(&path, &bytes).await.unwrap();
            cases.push((role, MediaSource::Path(path), bytes, Some(expected)));
        }

        for (role, source, expected_bytes, expected_type) in cases {
            let (wire_role, numbered, image) = match role {
                AssetRole::ReferenceAudio | AssetRole::ReferenceAudioIdentity => {
                    ("referenceAudio".into(), false, false)
                }
                AssetRole::ReferenceVideo => ("referenceVideo".into(), false, false),
                AssetRole::ReferenceAudioSlot(slot) => {
                    (format!("referenceAudio{slot}"), true, false)
                }
                AssetRole::ReferenceVideoSlot(slot) => {
                    (format!("referenceVideo{slot}"), true, false)
                }
                AssetRole::KeyframeImage(slot) => (format!("keyframeImage{slot}"), false, true),
                _ => unreachable!(),
            };
            let model = if numbered {
                "minimax-h3-ref2va-fp8_r2v"
            } else {
                "ltx23-22b-fp8_i2v"
            };
            let mut request = json!({"keyFrames":[{"modelID":model}]});
            let session = client.projects.inner.client.rest.request_session();
            tokio::time::timeout(
                WATCHDOG,
                client.projects.process_assets(
                    &session,
                    "MIME-FIXTURE",
                    &[(role.clone(), source)],
                    &mut request,
                    true,
                ),
            )
            .await
            .unwrap()
            .unwrap();
            let Call::Registration(path, query) = calls.recv().await.unwrap() else {
                panic!("asset must register before uploading")
            };
            assert_eq!(
                path,
                if image {
                    "/v1/image/uploadUrl"
                } else {
                    "/v1/media/uploadUrl"
                }
            );
            assert_eq!(query.get("contentType").map(String::as_str), expected_type);
            assert_eq!(
                query.get("type").unwrap(),
                if numbered {
                    if wire_role.starts_with("referenceAudio") {
                        "referenceAudio"
                    } else {
                        "referenceVideo"
                    }
                } else {
                    &wire_role
                }
            );
            assert_eq!(query.get("id"), numbered.then_some(&wire_role));
            let Call::Upload(headers, bytes) = calls.recv().await.unwrap() else {
                panic!("registration must be followed by the byte upload")
            };
            assert_eq!(
                headers
                    .get("content-type")
                    .map(|value| value.to_str().unwrap()),
                expected_type
            );
            assert_eq!(bytes.as_ref(), expected_bytes);
            for name in ["api-key", "authorization", "cookie"] {
                assert!(!headers.contains_key(name));
            }
            let frame = &request["keyFrames"][0];
            if image {
                assert!(frame.get(format!("{wire_role}ContentType")).is_none());
            } else {
                assert_eq!(
                    frame
                        .get(format!("{wire_role}ContentType"))
                        .and_then(Value::as_str),
                    expected_type
                );
                if matches!(role, AssetRole::ReferenceAudioIdentity) {
                    assert_eq!(
                        frame["referenceAudioIdentityContentType"].as_str(),
                        expected_type
                    );
                }
            }
        }
        client.close().await.unwrap();
        server.abort();
    }
}
