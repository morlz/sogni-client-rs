use super::*;
impl ProjectsApi {
    /// Create and submit an image, video, or audio generation project.
    pub async fn create(&self, mut request: ProjectRequest) -> Result<Project> {
        for (role, _) in &request.assets {
            mark_asset_param(&mut request.params, role);
        }
        validate_project_params(&request.params)?;
        let model_id = required_str(&request.params, "modelId")?;
        validate_asset_roles(model_id, &request.assets)?;
        let options = self.get_model_options(model_id, false).await?;
        let project_id = new_id();
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
        let mut wire =
            build_job_request(&project_id, &request.params, &options, workload.as_ref())?;
        self.process_assets(&project_id, &request.assets, &mut wire)
            .await?;
        let project = Project::new(
            project_id.clone(),
            Value::Object(request.params),
            false,
            Arc::downgrade(&self.inner),
        );
        self.inner
            .projects
            .write()
            .insert(project_id.clone(), project.clone());
        if let Err(error) = self.inner.client.send_socket("jobRequest", &wire).await {
            self.inner.projects.write().remove(&project_id);
            return Err(error);
        }
        self.inner
            .events
            .emit("projectCreated", json!({"projectId": project_id}));
        Ok(project)
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
