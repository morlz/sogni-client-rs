use super::*;

#[derive(Default)]
pub(in crate::projects) struct SubmissionRecovery {
    pub(in crate::projects) unadmitted: HashMap<String, Value>,
    pub(in crate::projects) awaiting: HashSet<String>,
    pub(in crate::projects) submitted_at: HashMap<String, DateTime<Utc>>,
    recheck_epoch: u64,
}

impl ProjectsApi {
    /// Only explicit project-wide 1001 refusals can be retried automatically:
    /// the server did not admit or charge them. Unknown writes stay uncertain.
    pub(in crate::projects) fn resubmit_if_restarting(&self, data: &Value) -> bool {
        let code = data
            .get("error")
            .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()));
        if code != Some(1001) || data.get("imgID").and_then(Value::as_str).is_some() {
            return false;
        }
        let Some(id) = data
            .get("jobID")
            .and_then(Value::as_str)
            .map(str::to_uppercase)
        else {
            return false;
        };
        let Some(project) = self
            .inner
            .projects
            .read()
            .get(&id)
            .cloned()
            .filter(|project| !project.status().is_finished())
        else {
            return false;
        };
        let request = {
            let mut state = self.inner.submission.lock();
            if state.awaiting.contains(&id) {
                return true;
            }
            let Some(request) = state.unadmitted.remove(&id) else {
                return false;
            };
            state.awaiting.insert(id.clone());
            request
        };
        let mut events = self.inner.client.subscribe();
        let session = self.inner.client.rest.auth_updates();
        let api = self.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(60), async {
                loop {
                    let event = events.recv().await.map_err(|_| Error::Closed)?;
                    if session.has_changed().unwrap_or(true) {
                        return Err(Error::Closed);
                    }
                    if event.name == "disconnected" {
                        return Err(Error::Closed);
                    }
                    if event.name != "connected" {
                        continue;
                    }
                    if project.status().is_finished() {
                        return Ok(());
                    }
                    // Record before sending: an acknowledgement can arrive
                    // immediately after the socket write.
                    api.inner
                        .submission
                        .lock()
                        .unadmitted
                        .insert(id.clone(), request.clone());
                    api.inner.client.send_socket("jobRequest", &request).await?;
                    api.inner
                        .submission
                        .lock()
                        .submitted_at
                        .insert(id.clone(), Utc::now());
                    return Ok(());
                }
            })
            .await;
            api.inner.submission.lock().awaiting.remove(&id);
            if matches!(result, Ok(Ok(()))) {
                api.schedule_recheck(RECENTLY_CREATED_GRACE);
            } else {
                api.inner.submission.lock().unadmitted.remove(&id);
                let error = json!({"code":1001,"message":"The server restarted before accepting this project. Please try again."});
                project.update(
                    |state| {
                        if !state.status.is_finished() {
                            state.status = ProjectStatus::Failed;
                            state.error = Some(error.clone());
                        }
                    },
                    &["status", "error"],
                );
                api.inner.events.emit(
                    "project",
                    json!({"type":"error", "projectId":id,"error":error}),
                );
            }
        });
        true
    }

    pub(in crate::projects) fn schedule_recheck(&self, delay: Duration) {
        let epoch = {
            let mut state = self.inner.submission.lock();
            state.recheck_epoch = state.recheck_epoch.wrapping_add(1);
            state.recheck_epoch
        };
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            tokio::time::sleep(delay + Duration::from_millis(250)).await;
            if let Some(inner) = weak.upgrade() {
                if inner.submission.lock().recheck_epoch != epoch {
                    return;
                }
                let api = ProjectsApi { inner };
                if api.sync("recheck").await.is_err() {
                    tracing::debug!("project recheck was inconclusive");
                }
            }
        });
    }
}
