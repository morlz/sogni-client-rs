use super::*;

impl Project {
    /// Current explanation for the remaining queued results, even in a running batch.
    #[must_use]
    pub fn waiting_reason(&self) -> Option<WaitingReason> {
        self.inner.state.read().waiting_reason.clone()
    }

    /// Complete current queue list; entries do not create jobs before assignment.
    #[must_use]
    pub fn job_waiting_reasons(&self) -> Vec<JobWaitingReason> {
        self.inner.state.read().job_waiting_reasons.clone()
    }

    pub(in crate::projects) fn queue_revision(&self) -> u64 {
        self.inner.state.read().queue_revision
    }

    pub(in crate::projects) fn advance_queue_revision(&self) -> u64 {
        let mut state = self.inner.state.write();
        state.queue_revision = state.queue_revision.wrapping_add(1);
        state.queue_revision
    }

    pub(in crate::projects) fn apply_queue_snapshot(
        &self,
        raw: &Value,
        expected_revision: u64,
    ) -> bool {
        self.set_queue_state(
            raw.get("waitingReason").unwrap_or(&Value::Null),
            raw.get("jobWaitingReasons").unwrap_or(&Value::Null),
            Some(expected_revision),
        )
    }

    pub(in crate::projects) fn receive_queue(&self, raw: &Value) {
        let revision = {
            let mut state = self.inner.state.write();
            if state.status.is_finished() {
                return;
            }
            state.queue_revision = state.queue_revision.wrapping_add(1);
            state.queue_revision
        };
        self.apply_queue_snapshot(raw, revision);
    }

    pub(in crate::projects) fn invalidate_queue(&self) {
        let revision = {
            let mut state = self.inner.state.write();
            state.queue_revision = state.queue_revision.wrapping_add(1);
            state.queue_revision
        };
        self.set_queue_state(&Value::Null, &json!([]), Some(revision));
    }

    pub(in crate::projects) fn clear_job_queue(&self, id: &str, index: Option<u64>) {
        let index = index.or_else(|| self.job(id).and_then(|job| job.job_index()));
        let (revision, reason, rows) = {
            let mut state = self.inner.state.write();
            state.queue_revision = state.queue_revision.wrapping_add(1);
            let rows = state
                .job_waiting_reasons
                .iter()
                .filter(|row| {
                    !row.img_id
                        .as_deref()
                        .is_some_and(|known| known.eq_ignore_ascii_case(id))
                        && Some(row.job_index) != index
                })
                .cloned()
                .collect::<Vec<_>>();
            if rows.len() == state.job_waiting_reasons.len()
                && state
                    .waiting_reason
                    .as_ref()
                    .is_none_or(|reason| reason.reason != "payment_pending")
            {
                return;
            }
            (
                state.queue_revision,
                rows.first().map(|row| row.waiting_reason.clone()),
                rows,
            )
        };
        self.set_queue_state(&json!(reason), &json!(rows), Some(revision));
    }

    fn set_queue_state(&self, reason: &Value, rows: &Value, expected: Option<u64>) -> bool {
        let has_rows = rows.as_array().is_some_and(|rows| !rows.is_empty());
        let jobs = self.jobs();
        let event = {
            let mut state = self.inner.state.write();
            if expected.is_some_and(|revision| revision != state.queue_revision) {
                return false;
            }
            let mut reason = (!state.status.is_finished())
                .then(|| crate::projects::queue::normalize_waiting_reason(reason))
                .flatten();
            let rows = if state.status.is_finished() {
                Vec::new()
            } else {
                crate::projects::queue::normalize_job_waiting_reasons(
                    rows,
                    expected_jobs(&state.params),
                )
                .into_iter()
                .filter(|row| {
                    jobs.iter()
                        .find(|job| matches_row(row, job))
                        .is_none_or(|job| job.status() == JobStatus::Pending)
                })
                .collect::<Vec<_>>()
            };
            // A supplied per-result list is authoritative even when every row was invalid or already running.
            if has_rows {
                reason = rows.first().map(|row| row.waiting_reason.clone());
            }
            for job in &jobs {
                let next = if job.status() == JobStatus::Pending {
                    rows.iter()
                        .find(|row| matches_row(row, job))
                        .map(|row| row.waiting_reason.clone())
                } else {
                    None
                };
                if job.waiting_reason() != next {
                    job.update(|state| state.waiting_reason = next, &["waitingReason"]);
                }
            }
            if state.waiting_reason == reason && state.job_waiting_reasons == rows {
                return false;
            }
            state.waiting_reason = reason;
            state.job_waiting_reasons = rows;
            state.queue_revision = state.queue_revision.wrapping_add(1);
            ProjectQueueChanged {
                project_id: state.id.clone(),
                waiting_reason: state.waiting_reason.clone(),
                job_waiting_reasons: state.job_waiting_reasons.clone(),
            }
        };
        self.notify("updated", json!(["waitingReason", "jobWaitingReasons"]));
        if let Some(api) = self.inner.api.upgrade() {
            api.events.emit("queueChanged", json!(event));
        }
        true
    }
}

fn matches_row(row: &JobWaitingReason, job: &Job) -> bool {
    row.img_id
        .as_deref()
        .is_some_and(|id| id.eq_ignore_ascii_case(&job.id()))
        || job.job_index() == Some(row.job_index)
}
