use super::*;
impl ProjectsApi {
    /// Create and submit an image, video, or audio generation project.
    pub async fn create(&self, request: ProjectRequest) -> Result<Project> {
        self.create_with_id(&new_id(), request).await
    }

    /// Submit with an application-reserved UUID so its identity can be durably
    /// recorded before any external side effect. This does not make server
    /// submission idempotent: reconcile an uncertain submission before retrying.
    pub async fn create_with_id(
        &self,
        project_id: &str,
        request: ProjectRequest,
    ) -> Result<Project> {
        self.create_with_id_detailed(project_id, request)
            .await
            .map_err(ProjectSubmissionError::into_cause)
    }

    /// Submit with a phase-aware error so callers can distinguish failed asset
    /// preparation from uncertain generation submission. This never retries a
    /// generation request; callers must reconcile any failed `Send` phase.
    pub async fn create_with_id_detailed(
        &self,
        project_id: &str,
        mut request: ProjectRequest,
    ) -> std::result::Result<Project, ProjectSubmissionError> {
        let session = self.inner.client.rest.auth_updates();
        let (project_id, mut wire) = self
            .prepare_submission(project_id, &mut request)
            .await
            .map_err(|cause| ProjectSubmissionError::new(SubmissionPhase::Prepare, cause))?;
        self.process_assets(
            &project_id,
            &request.assets,
            &mut wire,
            matches!(
                request.params.get("type").and_then(Value::as_str),
                Some("video" | "audio")
            ),
        )
        .await
        .map_err(|cause| ProjectSubmissionError::new(SubmissionPhase::AssetUpload, cause))?;
        if session.has_changed().unwrap_or(true) {
            return Err(ProjectSubmissionError::new(
                SubmissionPhase::Prepare,
                Error::InvalidInput("The account changed. Select the upload again.".into()),
            ));
        }
        let project = Project::new(
            project_id.clone(),
            Value::Object(request.params),
            false,
            Arc::downgrade(&self.inner),
        );
        {
            let mut projects = self.inner.projects.write();
            if projects.contains_key(&project_id) {
                return Err(ProjectSubmissionError::new(
                    SubmissionPhase::Prepare,
                    Error::InvalidInput("project id is already tracked".into()),
                ));
            }
            projects.insert(project_id.clone(), project.clone());
        }
        self.inner
            .submission
            .lock()
            .unadmitted
            .insert(project_id.clone(), wire.clone());
        if let Err(cause) = self.inner.client.send_socket("jobRequest", &wire).await {
            self.inner.projects.write().remove(&project_id);
            self.inner.submission.lock().unadmitted.remove(&project_id);
            return Err(ProjectSubmissionError::new(SubmissionPhase::Send, cause));
        }
        self.inner
            .events
            .emit("projectCreated", json!({"projectId": project_id}));
        Ok(project)
    }

    async fn prepare_submission(
        &self,
        project_id: &str,
        request: &mut ProjectRequest,
    ) -> Result<(String, Value)> {
        let project_id = normalize_project_id(project_id)?;
        if request.params.get("type").and_then(Value::as_str) == Some("video") {
            validation::reject_retired_output_scale(&request.params)?;
        }
        if self.inner.projects.read().contains_key(&project_id) {
            return Err(Error::InvalidInput("project id is already tracked".into()));
        }
        for (role, _) in &request.assets {
            mark_asset_param(&mut request.params, role);
        }
        normalize_inline_mask(request)?;
        wire::normalize_utility_params(&mut request.params);
        validate_project_params(&request.params)?;
        let model_id = required_str(&request.params, "modelId")?;
        validate_asset_roles(model_id, &request.assets)?;
        let options = self.get_model_options(model_id, false).await?;
        let app_source = request
            .params
            .get("appSource")
            .and_then(Value::as_str)
            .or_else(|| self.inner.client.app_source())
            .map(ToOwned::to_owned);
        if let Some(app_source) = &app_source {
            request.params.insert("appSource".into(), json!(app_source));
        }
        let workload = self
            .inner
            .client
            .resolve_workload_attribution(request.attribution.as_ref(), Some(&project_id));
        let wire = build_job_request(&project_id, &request.params, &options, workload.as_ref())?;
        Ok((project_id, wire))
    }

    pub async fn cancel(&self, project_id: &str) -> Result<()> {
        require_nonempty(project_id, "project_id")?;
        cancel_project(&self.inner, &project_id.to_uppercase()).await
    }

    pub async fn get(&self, project_id: &str) -> Result<Value> {
        require_nonempty(project_id, "project_id")?;
        let response = self
            .inner
            .client
            .rest
            .get(&format!("/v1/projects/{}", path_segment(project_id)), None)
            .await?;
        response
            .pointer("/data/project")
            .cloned()
            .ok_or_else(|| Error::Protocol("project response missing data.project".into()))
    }
}

fn normalize_inline_mask(request: &mut ProjectRequest) -> Result<()> {
    if request.params.get("type").and_then(Value::as_str) != Some("image")
        || !request
            .params
            .get("modelId")
            .and_then(Value::as_str)
            .is_some_and(is_gpt_image_model)
    {
        return Ok(());
    }
    let Some(uri) = request
        .params
        .get("gptImageMaskUrl")
        .and_then(Value::as_str)
        .filter(|uri| uri.starts_with("data:"))
    else {
        return Ok(());
    };
    if request
        .params
        .get("gptImageMask")
        .is_some_and(|value| !value.is_null() && value != false)
    {
        return Err(Error::InvalidInput(
            "Provide one GPT Image mask, not both media and URL".into(),
        ));
    }
    let invalid =
        || Error::InvalidInput("GPT Image mask must be a PNG data URI smaller than 50 MB".into());
    let encoded = uri
        .strip_prefix("data:image/png;base64,")
        .filter(|encoded| {
            !encoded.is_empty() && encoded.len() < (50 * 1024 * 1024_usize * 4).div_ceil(3)
        })
        .ok_or_else(invalid)?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| invalid())?;
    request.assets.push((
        AssetRole::GptImageMask,
        MediaSource::named_bytes(bytes, "mask.png", "image/png"),
    ));
    request.params.remove("gptImageMaskUrl");
    request.params.insert("gptImageMask".into(), json!(true));
    Ok(())
}

pub(super) fn normalize_project_id(id: &str) -> Result<String> {
    uuid::Uuid::parse_str(id)
        .map(|id| id.hyphenated().to_string().to_uppercase())
        .map_err(|_| Error::InvalidInput("project id must be a UUID".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_project_identity_is_canonical_and_rejects_paths() {
        assert_eq!(
            normalize_project_id("a3f360cb-7f84-4d56-b360-a047e7cfb3cd").unwrap(),
            "A3F360CB-7F84-4D56-B360-A047E7CFB3CD"
        );
        assert!(normalize_project_id("../projects/another").is_err());
    }

    #[test]
    fn inline_gpt_mask_becomes_an_uploaded_reference_mask_before_serialization() {
        let mut request = ProjectRequest::image("gpt-image-2.5-flare", "edit")
            .param("contextImage1", true)
            .param("gptImageMaskUrl", "data:image/png;base64,cG5n");
        normalize_inline_mask(&mut request).unwrap();
        assert!(!request.params.contains_key("gptImageMaskUrl"));
        assert_eq!(request.params["gptImageMask"], true);
        assert!(matches!(&request.assets[0].0, AssetRole::GptImageMask));
        for uri in ["data:image/jpeg;base64,cG5n", "data:image/png;base64,@bad"] {
            let mut request =
                ProjectRequest::image("gpt-image-2.5-flare", "edit").param("gptImageMaskUrl", uri);
            assert!(normalize_inline_mask(&mut request).is_err());
        }
    }
}
