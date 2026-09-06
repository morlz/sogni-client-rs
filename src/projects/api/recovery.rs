use super::*;

impl ProjectsApi {
    /// Rehydrate one application-owned project without submitting new work.
    /// Keep the same app id across restarts for active-project recovery.
    pub async fn recover_project(&self, project_id: &str) -> Result<Project> {
        require_nonempty(project_id, "project_id")?;
        let project_id = project_id.to_uppercase();
        let tracked = { self.inner.projects.read().get(&project_id).cloned() };
        if let Some(project) = tracked.filter(incomplete_results) {
            let raw = self.get_status(&project_id).await?;
            if is_llm_recovery(&raw) {
                return Err(Error::Protocol(
                    "recovered project type does not match".into(),
                ));
            }
            replay_recovered(&project, &raw, false);
            self.resolve_recovered_urls(&project).await;
            return Ok(project);
        }
        self.sync("application-recovery").await?;
        let tracked = { self.inner.projects.read().get(&project_id).cloned() };
        if let Some(project) = tracked {
            return Ok(project);
        }
        let raw = self.get_status(&project_id).await?;
        if recovery_id(&raw).as_deref() != Some(&project_id) || is_llm_recovery(&raw) {
            return Err(Error::Protocol(
                "recovered project identity does not match".into(),
            ));
        }
        let project = Project::new(
            project_id.clone(),
            recovered_params(&raw),
            true,
            Arc::downgrade(&self.inner),
        );
        replay_recovered(&project, &raw, false);
        self.resolve_recovered_urls(&project).await;
        self.inner
            .projects
            .write()
            .insert(project_id, project.clone());
        Ok(project)
    }

    /// Reconcile tracked projects with the socket server's durable snapshot.
    ///
    /// The returned JSON follows the JavaScript and Python `ProjectSyncResult`
    /// shape and is also emitted as `projectsSynced`.
    pub async fn sync(&self, reason: &str) -> Result<Value> {
        let requested_at = Utc::now();
        let query = json!({"appId": self.inner.client.app_id()});
        let snapshot = self
            .inner
            .client
            .socket_get("/api/v1/artist/projects/sync", Some(&query))
            .await?;
        let _sync_guard = self.inner.sync_lock.lock().await;
        self.reconcile(snapshot, reason, requested_at).await
    }

    /// Return in-flight projects owned by other app instances for this account.
    pub async fn list_projects_elsewhere(&self) -> Result<Vec<Value>> {
        let response = self
            .inner
            .client
            .socket_get("/api/v1/artist/projects/sync", None)
            .await?;
        Ok(response
            .get("activeProjects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|project| {
                !is_llm_recovery(project)
                    && project.get("id").and_then(Value::as_str).is_some()
                    && project
                        .get("appId")
                        .and_then(Value::as_str)
                        .is_some_and(|app_id| app_id != self.inner.client.app_id())
            })
            .cloned()
            .collect())
    }

    /// Resolve project ids absent from the last recovery snapshot.
    ///
    /// A 404 is retried because the durable REST record is written
    /// asynchronously. Remaining ids are checked against the socket's live
    /// registry before they are classified as lost.
    pub async fn resolve_missing<S: AsRef<str>>(
        &self,
        project_ids: &[S],
        options: Option<ResolveMissingOptions>,
    ) -> BTreeMap<String, ProjectResolution> {
        let options = options.unwrap_or_default();
        let attempts = options.attempts.max(1);
        let mut seen = HashSet::new();
        let mut pending = project_ids
            .iter()
            .map(|id| id.as_ref().to_owned())
            .filter(|id| seen.insert(id.clone()))
            .collect::<Vec<_>>();
        let mut result = BTreeMap::new();

        for attempt in 0..attempts {
            if pending.is_empty() {
                break;
            }
            if attempt != 0 && !options.retry_delay.is_zero() {
                tokio::time::sleep(options.retry_delay).await;
            }
            let mut still_missing = Vec::new();
            for project_id in pending {
                match self.get_status(&project_id.to_uppercase()).await {
                    Ok(project) => {
                        let resolution =
                            if project.get("finished").and_then(Value::as_bool) == Some(true) {
                                ProjectResolution::Finished { project }
                            } else {
                                ProjectResolution::Active
                            };
                        result.insert(project_id, resolution);
                    }
                    Err(Error::Api(error)) if error.status == 404 => {
                        still_missing.push(project_id);
                    }
                    Err(error) => {
                        let _ = error;
                        tracing::debug!("missing project lookup was inconclusive");
                        result.insert(
                            project_id,
                            ProjectResolution::Unknown {
                                error: "project status could not be verified".into(),
                            },
                        );
                    }
                }
            }
            pending = still_missing;
        }

        if !pending.is_empty() {
            let active = self.list_active_project_ids().await;
            classify_exhausted_404s(&mut result, pending, active.as_ref());
        }
        result
    }

    async fn list_active_project_ids(&self) -> Option<HashSet<String>> {
        let response = self
            .inner
            .client
            .socket_get("/api/v1/artist/projects/active", None)
            .await
            .ok()?;
        active_project_ids(&response)
    }

    async fn reconcile(
        &self,
        snapshot: Value,
        reason: &str,
        requested_at: DateTime<Utc>,
    ) -> Result<Value> {
        let mut active = Vec::new();
        let mut completed = Vec::new();
        let mut lost = Vec::new();
        let mut unverified = Vec::new();
        let mut recovered_active = Vec::new();
        let mut recovered_completed = Vec::new();
        let mut seen = HashSet::new();

        for raw in recovery_records(&snapshot, "activeProjects") {
            let Some(id) = recovery_id(raw) else {
                continue;
            };
            if is_llm_recovery(raw) || !seen.insert(id.clone()) {
                continue;
            }
            let tracked = { self.inner.projects.read().get(&id).cloned() };
            if let Some(project) = tracked {
                if !project.status().is_finished() {
                    replay_recovered(&project, raw, false);
                    active.push(json!(id));
                }
            } else {
                let project = Project::new(
                    id.clone(),
                    recovered_params(raw),
                    true,
                    Arc::downgrade(&self.inner),
                );
                self.inner.projects.write().insert(id, project.clone());
                replay_recovered(&project, raw, false);
                recovered_active.push(raw.clone());
            }
        }

        for raw in recovery_records(&snapshot, "unclaimedCompletedProjects") {
            let Some(id) = recovery_id(raw) else {
                continue;
            };
            if is_llm_recovery(raw) || !seen.insert(id.clone()) {
                continue;
            }
            let tracked = { self.inner.projects.read().get(&id).cloned() };
            if let Some(project) = tracked {
                if !project.status().is_finished() || incomplete_results(&project) {
                    replay_recovered(&project, raw, true);
                    self.resolve_recovered_urls(&project).await;
                    completed.push(json!(id));
                }
                continue;
            }
            let first_recovery = {
                self.inner
                    .recovered_completed_ids
                    .write()
                    .insert(id.clone())
            };
            if !first_recovery {
                continue;
            }
            let project = Project::new(
                id.clone(),
                recovered_params(raw),
                true,
                Arc::downgrade(&self.inner),
            );
            self.inner.projects.write().insert(id, project.clone());
            replay_recovered(&project, raw, true);
            self.resolve_recovered_urls(&project).await;
            let mut record = raw.as_object().cloned().unwrap_or_default();
            record.insert("resultUrls".into(), json!(project.result_urls()));
            recovered_completed.push(Value::Object(record));
        }

        let cutoff = requested_at
            - chrono::Duration::from_std(RECENTLY_CREATED_GRACE)
                .expect("recovery grace duration is representable");
        let missing = self
            .inner
            .projects
            .read()
            .values()
            .filter(|project| {
                let snapshot = project.snapshot();
                (!snapshot.status.is_finished() || incomplete_results(project))
                    && !seen.contains(&snapshot.id)
                    && snapshot.started_at <= cutoff
            })
            .cloned()
            .collect::<Vec<_>>();

        if !missing.is_empty() {
            let ids = missing.iter().map(Project::id).collect::<Vec<_>>();
            let resolutions = self.resolve_missing(&ids, None).await;
            for project in missing {
                if project.status().is_finished() && !incomplete_results(&project) {
                    continue;
                }
                match resolutions.get(&project.id()) {
                    Some(ProjectResolution::Finished { project: raw }) => {
                        replay_recovered(&project, raw, false);
                        self.resolve_recovered_urls(&project).await;
                        completed.push(json!(project.id()));
                    }
                    Some(ProjectResolution::Active) => active.push(json!(project.id())),
                    Some(ProjectResolution::Lost) => {
                        let error = project_lost_payload();
                        let mut marked_lost = false;
                        project.update(
                            |state| {
                                if !state.status.is_finished() {
                                    state.status = ProjectStatus::Failed;
                                    state.error = Some(error.clone());
                                    marked_lost = true;
                                }
                            },
                            &["status", "error"],
                        );
                        if marked_lost {
                            self.inner.events.emit(
                                "project",
                                json!({"type": "error", "projectId": project.id(), "error": error}),
                            );
                            lost.push(json!(project.id()));
                        } else {
                            unverified.push(json!(project.id()));
                        }
                    }
                    Some(ProjectResolution::Unknown { .. }) | None => {
                        unverified.push(json!(project.id()));
                    }
                }
            }
        }

        if !recovered_active.is_empty() {
            self.inner.events.emit(
                ACTIVE_PROJECTS_RECOVERED_EVENT,
                Value::Array(recovered_active.clone()),
            );
        }
        if !recovered_completed.is_empty() {
            self.inner.events.emit(
                COMPLETED_PROJECTS_RECOVERED_EVENT,
                Value::Array(recovered_completed.clone()),
            );
        }
        let result = json!({
            "reason": reason,
            "snapshot": snapshot,
            "active": active,
            "completed": completed,
            "lost": lost,
            "unverified": unverified,
            "recoveredActive": recovered_active,
            "recoveredCompleted": recovered_completed,
        });
        self.inner.events.emit("projectsSynced", result.clone());
        Ok(result)
    }

    async fn resolve_recovered_urls(&self, project: &Project) {
        for job in project.jobs() {
            if job.status() == JobStatus::Completed
                && !job.is_withheld()
                && job.result_url().is_none()
                && job.get_result_url().await.is_err()
            {
                tracing::debug!("recovered result URL unavailable");
            }
        }
    }
}

fn incomplete_results(project: &Project) -> bool {
    let snapshot = project.snapshot();
    if snapshot.status != ProjectStatus::Completed {
        return false;
    }
    let expected = u64::from(expected_jobs(&snapshot.params));
    snapshot.jobs.len() < usize::try_from(expected).unwrap_or(usize::MAX)
        || snapshot.jobs.iter().any(|job| {
            !job.status.is_finished()
                || (job.status == JobStatus::Completed
                    && (!job.is_nsfw || job.nsfw_detected)
                    && job.result_url.is_none())
        })
}

fn recovery_records<'a>(snapshot: &'a Value, field: &str) -> impl Iterator<Item = &'a Value> {
    snapshot
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn recovery_id(raw: &Value) -> Option<String> {
    raw.get("id").and_then(Value::as_str).map(str::to_uppercase)
}

fn active_project_ids(response: &Value) -> Option<HashSet<String>> {
    response
        .get("projects")?
        .as_array()?
        .iter()
        .map(|project| project.get("id")?.as_str().map(str::to_uppercase))
        .collect()
}

fn classify_exhausted_404s(
    result: &mut BTreeMap<String, ProjectResolution>,
    pending: Vec<String>,
    active: Option<&HashSet<String>>,
) {
    for project_id in pending {
        let resolution = match active {
            Some(active) if active.contains(&project_id.to_uppercase()) => {
                ProjectResolution::Active
            }
            Some(_) => ProjectResolution::Lost,
            None => ProjectResolution::Unknown {
                error: "project status could not be verified".into(),
            },
        };
        result.insert(project_id, resolution);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod result_tests;
