use super::*;

#[derive(Default)]
pub(in crate::projects) struct SubmissionRecovery {
    pub(in crate::projects) unadmitted: HashMap<String, Value>,
    pub(in crate::projects) awaiting: HashSet<String>,
    pub(in crate::projects) submitted_at: HashMap<String, DateTime<Utc>>,
    /// Connection generation and account session of a completed socket write.
    pub(in crate::projects) sent_on: HashMap<String, (u64, u64)>,
    pub(super) handled_session: Option<u64>,
    recheck_epoch: u64,
}

impl SubmissionRecovery {
    pub(super) fn invalidate_rechecks(&mut self) {
        self.recheck_epoch = self.recheck_epoch.wrapping_add(1);
    }
    pub(in crate::projects) fn observed(&mut self, id: &str) {
        self.unadmitted.remove(id);
        self.sent_on.remove(id);
    }
}

impl ProjectsApi {
    /// Only after both owner status and the live registry confirmed absence.
    /// An uncertain write has no generation receipt and can never qualify.
    pub(super) async fn resend_undelivered(&self, project_id: &str) -> bool {
        let id = project_id.to_uppercase();
        if !self.inner.client.is_socket_authenticated() {
            return false;
        }
        let Some(generation) = self.inner.client.socket_generation() else {
            return false;
        };
        let guard = self.inner.client.rest.request_session();
        let session = guard.id();
        let project = self.inner.projects.read().get(&id).cloned();
        if project.as_ref().is_none_or(|project| {
            project.status().is_finished() || project.check_session().is_err()
        }) {
            return false;
        }
        let request = {
            let mut state = self.inner.submission.lock();
            let Some(&(sent_on, sent_session)) = state.sent_on.get(&id) else {
                return false;
            };
            if sent_on >= generation
                || sent_session != session
                || state.submitted_at.contains_key(&id)
                || state.awaiting.contains(&id)
            {
                return false;
            }
            let Some(request) = state.unadmitted.get(&id).cloned() else {
                return false;
            };
            // Claim before awaiting: simultaneous recovery passes must not resend twice.
            state.submitted_at.insert(id.clone(), Utc::now());
            state.sent_on.remove(&id);
            request
        };
        let result = guard
            .run(
                self.inner
                    .client
                    .send_socket_tracked_in_session("jobRequest", &request, session),
            )
            .await;
        if guard.check().is_err() {
            self.clear_previous_sessions();
            return false;
        }
        // Even a failed write may have arrived; leave a grace window and use
        // read-only recovery. Never issue a second automatic resend.
        self.inner
            .submission
            .lock()
            .submitted_at
            .insert(id.clone(), Utc::now());
        if let Ok(generation) = result {
            let mut state = self.inner.submission.lock();
            if state.unadmitted.contains_key(&id) {
                state.sent_on.insert(id, (generation, session));
            }
        }
        self.schedule_recheck(RECENTLY_CREATED_GRACE);
        true
    }

    pub(super) fn recently_resubmitted(&self, project_id: &str) -> bool {
        self.inner
            .submission
            .lock()
            .submitted_at
            .get(&project_id.to_uppercase())
            .is_some_and(|at| {
                Utc::now()
                    .signed_duration_since(*at)
                    .to_std()
                    .unwrap_or_default()
                    < RECENTLY_CREATED_GRACE
            })
    }

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
            .filter(|project| !project.status().is_finished() && project.check_session().is_ok())
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
        let mut events = self.inner.client.subscribe_scoped();
        let session = self.inner.client.rest.request_session();
        let auth_session = session.id();
        if project.auth_session() != auth_session {
            return false;
        }
        let api = self.clone();
        tokio::spawn(async move {
            let result = session
                .run(async {
                    tokio::time::timeout(Duration::from_secs(60), async {
                        loop {
                            let event = events.recv().await.map_err(|_| Error::Closed)?;
                            session.check()?;
                            if event.session.is_some_and(|owner| owner != auth_session) {
                                continue;
                            }
                            let event = event.event;
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
                            api.inner
                                .client
                                .send_socket_tracked_in_session(
                                    "jobRequest",
                                    &request,
                                    auth_session,
                                )
                                .await?;
                            session.check()?;
                            api.inner
                                .submission
                                .lock()
                                .submitted_at
                                .insert(id.clone(), Utc::now());
                            return Ok(());
                        }
                    })
                    .await
                    .map_err(|_| Error::Timeout("waiting to resubmit after restart".into()))?
                })
                .await;
            if session.check().is_err() {
                api.clear_previous_sessions();
                project.end_session();
                return;
            }
            api.inner.submission.lock().awaiting.remove(&id);
            if result.is_ok() {
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
        let session = self.inner.client.rest.request_session();
        let epoch = {
            let mut state = self.inner.submission.lock();
            state.recheck_epoch = state.recheck_epoch.wrapping_add(1);
            state.recheck_epoch
        };
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            if session
                .run(async {
                    tokio::time::sleep(delay + Duration::from_millis(250)).await;
                    Ok(())
                })
                .await
                .is_err()
            {
                return;
            }
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
