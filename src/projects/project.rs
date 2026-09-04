use super::*;
use crate::projects::api::ProjectsInner;

#[derive(Clone)]
pub struct Project {
    inner: Arc<ProjectInner>,
}

#[derive(Clone)]
pub(super) struct ProjectState {
    pub(super) id: String,
    pub(super) started_at: DateTime<Utc>,
    pub(super) recovered: bool,
    pub(super) params: Value,
    pub(super) media_type: String,
    pub(super) status: ProjectStatus,
    pub(super) error: Option<Value>,
    pub(super) eta: Option<DateTime<Utc>>,
    pub(super) queue_position: i64,
    pub(super) estimated_start_at: Option<DateTime<Utc>>,
    pub(super) queue_status: Option<String>,
}

pub(super) struct ProjectInner {
    pub(super) state: RwLock<ProjectState>,
    pub(super) jobs: RwLock<Vec<Job>>,
    pub(super) events: EventBus,
    pub(super) changed: Notify,
    pub(super) api: Weak<ProjectsInner>,
}

impl std::fmt::Debug for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Project")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl Project {
    pub(super) fn new(
        id: String,
        params: Value,
        recovered: bool,
        api: Weak<ProjectsInner>,
    ) -> Self {
        let media_type = params
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("image")
            .to_owned();
        Self {
            inner: Arc::new(ProjectInner {
                state: RwLock::new(ProjectState {
                    id,
                    started_at: Utc::now(),
                    recovered,
                    params,
                    media_type,
                    status: ProjectStatus::Pending,
                    error: None,
                    eta: None,
                    queue_position: -1,
                    estimated_start_at: None,
                    queue_status: None,
                }),
                jobs: RwLock::new(Vec::new()),
                events: EventBus::default(),
                changed: Notify::new(),
                api,
            }),
        }
    }

    #[must_use]
    pub fn id(&self) -> String {
        self.inner.state.read().id.clone()
    }

    #[must_use]
    pub fn status(&self) -> ProjectStatus {
        self.inner.state.read().status
    }

    #[must_use]
    pub fn jobs(&self) -> Vec<Job> {
        self.inner.jobs.read().clone()
    }

    #[must_use]
    pub fn progress(&self) -> u8 {
        let expected = expected_jobs(&self.inner.state.read().params);
        let total = self
            .inner
            .jobs
            .read()
            .iter()
            .map(|job| u64::from(job.progress()))
            .sum::<u64>();
        if self.status() == ProjectStatus::Completed {
            100
        } else {
            (total / u64::from(expected.max(1))).min(100) as u8
        }
    }

    #[must_use]
    pub fn result_urls(&self) -> Vec<String> {
        self.inner
            .jobs
            .read()
            .iter()
            .filter_map(Job::result_url)
            .collect()
    }

    #[must_use]
    pub fn snapshot(&self) -> ProjectSnapshot {
        self.snapshot_after_state_clone(|| {})
    }

    fn snapshot_after_state_clone(&self, after_state_clone: impl FnOnce()) -> ProjectSnapshot {
        let state = {
            let state = self.inner.state.read();
            state.clone()
        };
        after_state_clone();

        let job_handles = self.inner.jobs.read().clone();
        let jobs = job_handles.iter().map(Job::snapshot).collect::<Vec<_>>();
        let progress = if state.status == ProjectStatus::Completed {
            100
        } else {
            let expected = expected_jobs(&state.params).max(1);
            let total = jobs
                .iter()
                .map(|job| u64::from(job_progress(job)))
                .sum::<u64>();
            (total / u64::from(expected)).min(100) as u8
        };
        let result_urls = jobs
            .iter()
            .filter_map(|job| job.result_url.clone())
            .collect();

        ProjectSnapshot {
            id: state.id.clone(),
            started_at: state.started_at,
            recovered: state.recovered,
            params: state.params.clone(),
            media_type: state.media_type.clone(),
            status: state.status,
            error: state.error.clone(),
            eta: state.eta,
            queue_position: state.queue_position,
            estimated_start_at: state.estimated_start_at,
            queue_status: state.queue_status.clone(),
            jobs,
            progress,
            result_urls,
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> EventReceiver {
        self.inner.events.subscribe()
    }

    /// Wait without cancelling the server-side render when the local timeout elapses.
    pub async fn wait_for_completion(&self, timeout: Option<Duration>) -> Result<Vec<String>> {
        let wait =
            async {
                loop {
                    let changed = self.inner.changed.notified();
                    let snapshot = self.snapshot();
                    match snapshot.status {
                        ProjectStatus::Completed => {
                            let expected = expected_jobs(&snapshot.params) as usize;
                            if snapshot.jobs.len() >= expected
                                && snapshot.jobs.iter().all(|job| job.status.is_finished())
                            {
                                return Ok(snapshot.result_urls);
                            }
                        }
                        ProjectStatus::Failed => {
                            return Err(ProjectError::from_payload(snapshot.error.unwrap_or_else(
                                || json!({"code": 0, "message": "Project failed"}),
                            ))
                            .into());
                        }
                        ProjectStatus::Canceled => {
                            return Err(ProjectError::from_payload(json!({
                                "code": 5004,
                                "message": "Project was canceled",
                            }))
                            .into());
                        }
                        _ => {}
                    }
                    changed.await;
                }
            };
        if let Some(timeout) = timeout {
            tokio::time::timeout(timeout, wait)
                .await
                .map_err(|_| Error::Timeout(format!("project {} did not finish", self.id())))?
        } else {
            wait.await
        }
    }

    pub async fn cancel(&self) -> Result<()> {
        let api = self.inner.api.upgrade().ok_or(Error::Closed)?;
        cancel_project(&api, &self.id()).await
    }

    #[must_use]
    pub fn job(&self, id: &str) -> Option<Job> {
        let id = id.to_uppercase();
        self.inner
            .jobs
            .read()
            .iter()
            .find(|job| job.id() == id)
            .cloned()
    }

    pub(super) fn ensure_job(&self, id: &str) -> Job {
        if let Some(job) = self.job(id) {
            return job;
        }
        let state = self.inner.state.read();
        let step_count = number(state.params.get("steps")).unwrap_or(0.0);
        let output_format = state
            .params
            .get("outputFormat")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let job = Job::new(
            JobSnapshot::pending(id.to_owned(), state.id.clone(), step_count),
            self.inner
                .api
                .upgrade()
                .expect("project API lives while project is tracked")
                .client
                .clone(),
            Arc::downgrade(&self.inner),
            state.media_type.clone(),
            output_format,
        );
        drop(state);
        self.inner.jobs.write().push(job.clone());
        self.notify("jobStarted", json!({"jobId": id}));
        job
    }

    pub(super) fn update(&self, apply: impl FnOnce(&mut ProjectState), keys: &[&str]) {
        apply(&mut self.inner.state.write());
        self.notify("updated", json!(keys));
    }

    pub(super) fn notify(&self, name: &str, data: Value) {
        self.inner.events.emit(name, data);
        self.inner.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread, time::Duration};

    use super::*;

    #[test]
    fn snapshot_releases_state_before_collecting_jobs() {
        let project = Project::new(
            "PROJECT".into(),
            json!({"type": "image", "numberOfMedia": 1}),
            false,
            Weak::new(),
        );
        let jobs_guard = project.inner.jobs.write();
        let (state_cloned_tx, state_cloned_rx) = mpsc::sync_channel(0);
        let (snapshot_tx, snapshot_rx) = mpsc::sync_channel(1);
        let snapshot_project = project.clone();
        let snapshot_thread = thread::spawn(move || {
            let snapshot = snapshot_project.snapshot_after_state_clone(|| {
                state_cloned_tx.send(()).expect("test receiver is alive");
            });
            snapshot_tx.send(snapshot).expect("test receiver is alive");
        });

        state_cloned_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("snapshot reached job collection");
        let (writer_done_tx, writer_done_rx) = mpsc::sync_channel(1);
        let writer_project = project.clone();
        let writer_thread = thread::spawn(move || {
            writer_project.update(|state| state.queue_position = 7, &["queuePosition"]);
            writer_done_tx.send(()).expect("test receiver is alive");
        });

        writer_done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a queued state writer was blocked while snapshot waited on the jobs lock");
        drop(jobs_guard);

        let snapshot = snapshot_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("snapshot completes after the jobs lock is released");
        snapshot_thread
            .join()
            .expect("snapshot thread did not panic");
        writer_thread.join().expect("writer thread did not panic");
        assert_eq!(snapshot.queue_position, -1);
        assert_eq!(project.inner.state.read().queue_position, 7);
    }
}
