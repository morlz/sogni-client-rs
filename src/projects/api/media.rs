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
            if matches!(role, AssetRole::StartingImage) {
                self.upload_guide_image(project_id, &new_id(), source)
                    .await?;
                continue;
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
            // Image uploads advertise their MIME in the resource registration
            // and PUT only. The worker payload annotates video assets alone.
            if let (Some(keyframe), Some(content_type)) = (
                request
                    .pointer_mut("/keyFrames/0")
                    .filter(|_| annotate_video),
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
