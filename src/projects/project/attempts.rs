use super::*;

impl Project {
    /// A render retains its position and handle when a replacement worker mints
    /// a new id. Index-less frames may claim only one explicitly waiting render.
    pub(in crate::projects) fn job_for_attempt(&self, id: &str, index: Option<u64>) -> Option<Job> {
        let _guard = self.inner.attempt_lock.lock();
        let id = id.to_uppercase();
        if self.inner.retired_attempts.read().contains(&id) {
            return None;
        }
        if let Some(job) = self.job(&id) {
            if self.inner.awaiting_reassignment.read().contains(&id) {
                return None;
            }
            return Some(job);
        }
        let jobs = self.jobs();
        let unfinished: Vec<_> = jobs
            .iter()
            .filter(|job| !job.status().is_finished())
            .collect();
        let reassigned = index
            .and_then(|index| {
                unfinished
                    .iter()
                    .find(|job| job.job_index() == Some(index))
                    .copied()
            })
            .or_else(|| {
                let waiting = self.inner.awaiting_reassignment.read();
                let candidates: Vec<_> = unfinished
                    .iter()
                    .filter(|job| job.job_index().is_none() && waiting.contains(&job.id()))
                    .copied()
                    .collect();
                (candidates.len() == 1).then(|| candidates[0])
            });
        if let Some(job) = reassigned {
            let previous = job.id();
            self.inner.awaiting_reassignment.write().remove(&previous);
            self.inner.retired_attempts.write().insert(previous);
            reset_attempt(job, Some(&id), index);
            self.notify("updated", json!(["jobs"]));
            return Some(job.clone());
        }
        Some(self.ensure_job(&id))
    }

    pub(in crate::projects) fn retry_job(&self, data: &Value) {
        let _guard = self.inner.attempt_lock.lock();
        if self.status().is_finished() {
            return;
        }
        let index = data.get("jobIndex").and_then(Value::as_u64);
        let job = data
            .get("imgID")
            .and_then(Value::as_str)
            .and_then(|id| self.job(id))
            .or_else(|| {
                index.and_then(|index| {
                    self.jobs()
                        .into_iter()
                        .find(|job| job.job_index() == Some(index))
                })
            });
        let Some(job) = job.filter(|job| !job.status().is_finished()) else {
            return;
        };
        self.inner.awaiting_reassignment.write().insert(job.id());
        reset_attempt(&job, None, index);
        self.notify("updated", json!(["jobs"]));
    }
}

fn reset_attempt(job: &Job, id: Option<&str>, index: Option<u64>) {
    job.suspend_processing_deadline();
    job.update(
        |state| {
            if let Some(id) = id {
                state.id = id.to_owned();
            }
            if let Some(index) = index {
                state.extra.insert("jobIndex".into(), json!(index));
            }
            // No media or timings from an abandoned attempt belong to its successor.
            state.status = JobStatus::Pending;
            state.step = 0.0;
            state.external_progress = None;
            state.worker_name = None;
            state.preview_url = None;
            state.result_url = None;
            state.last_frame_url = None;
            state.last_frame_key = None;
            state.output_format = None;
            state.provenance = None;
            state.seed = None;
            state.error = None;
            state.eta = None;
            state.eta_seconds = None;
            state.eta_range = None;
            state.extra.remove("preparation");
        },
        &[
            "id",
            "status",
            "step",
            "workerName",
            "previewUrl",
            "eta",
            "provenance",
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SogniClient;

    async fn project(count: u64) -> (SogniClient, Project) {
        let client = SogniClient::builder()
            .disable_socket(true)
            .build()
            .await
            .unwrap();
        let project = Project::new(
            "PROJECT".into(),
            json!({"type":"video", "numberOfMedia":count}),
            false,
            Arc::downgrade(&client.projects.inner),
        );
        (client, project)
    }

    #[tokio::test]
    async fn stable_index_keeps_handle_and_resets_only_abandoned_attempt() {
        let (client, project) = project(2).await;
        let first = project.job_for_attempt("OLD", Some(0)).unwrap();
        first.update(
            |state| {
                state.extra.insert("jobIndex".into(), json!(0));
                state.step = 9.0;
                state.status = JobStatus::Processing;
                state.provenance = Some(JobProvenance {
                    sha256: Some("a".repeat(64)),
                    ..Default::default()
                });
                state.preview_url = Some("old-preview".into());
            },
            &[],
        );
        let sibling = project.job_for_attempt("SIBLING", Some(1)).unwrap();
        sibling.update(
            |state| {
                state.extra.insert("jobIndex".into(), json!(1));
                state.step = 4.0;
            },
            &[],
        );
        // A disconnect requeue need not carry an explicit jobRetry frame.
        let replacement = project.job_for_attempt("NEW", Some(0)).unwrap();
        assert_eq!(first.id(), "NEW");
        assert_eq!(replacement.snapshot().step, 0.0);
        assert!(replacement.provenance().is_none());
        assert!(replacement.snapshot().preview_url.is_none());
        assert_eq!(sibling.snapshot().step, 4.0);
        assert_eq!(project.jobs().len(), 2);
        assert!(project.job_for_attempt("OLD", None).is_none());
        client.close().await.unwrap();
    }

    #[tokio::test]
    async fn indexless_reassignment_requires_one_announced_waiter() {
        let (client, project) = project(2).await;
        let old = project.ensure_job("OLD");
        let sibling = project.ensure_job("SIBLING");
        project.retry_job(&json!({"imgID":"OLD"}));
        assert_eq!(old.status(), JobStatus::Pending);
        assert!(project.job_for_attempt("OLD", None).is_none());
        project.job_for_attempt("NEW", None).unwrap();
        assert_eq!(old.id(), "NEW");
        assert_eq!(sibling.id(), "SIBLING");
        assert_eq!(project.jobs().len(), 2);
        project.retry_job(&json!({"imgID":"NEW"}));
        project.retry_job(&json!({"imgID":"SIBLING"}));
        project.job_for_attempt("AMBIGUOUS", None).unwrap();
        assert_eq!(old.id(), "NEW");
        assert_eq!(sibling.id(), "SIBLING");
        client.close().await.unwrap();
    }
}
