use super::*;
impl ProjectsApi {
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
        project_id: &str,
        assets: &[(AssetRole, MediaSource)],
        request: &mut Value,
    ) -> Result<()> {
        let model_id = request
            .pointer("/keyFrames/0/modelID")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut seen = std::collections::HashSet::new();
        for (role, source) in assets {
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
            let media = source.read().await?;
            let query = if role.is_media() {
                json!({
                    "jobId": project_id,
                    "type": wire_role,
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
            let upload = if role.is_media() {
                self.media_upload_url(&query).await?
            } else {
                self.upload_url(&query).await?
            };
            self.inner
                .client
                .rest
                .put_bytes(upload, media.data, media.content_type.as_deref())
                .await?;
            if let Some(keyframe) = request.pointer_mut("/keyFrames/0") {
                if let Some(content_type) = media.content_type {
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
        }
        Ok(())
    }
}
