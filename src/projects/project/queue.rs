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

    /// The current server offer, if all results are still eligible.
    #[must_use]
    pub fn pay_as_you_go(&self) -> Option<PayAsYouGoOffer> {
        self.inner.state.read().pay_as_you_go.clone()
    }

    /// Explicitly accept Premium Spark payment for this waiting project.
    pub async fn switch_to_premium_spark(
        &self,
        timeout: Option<Duration>,
    ) -> Result<PayAsYouGoSwitchResult> {
        self.check_session()?;
        let api = ProjectsApi {
            inner: self.inner.api.upgrade().ok_or(Error::Closed)?,
        };
        api.switch_project_payment(&self.id(), timeout, self.inner.session.clone())
            .await
    }

    pub(in crate::projects) fn clear_pay_as_you_go(&self) {
        let event = {
            let mut state = self.inner.state.write();
            state.queue_revision = state.queue_revision.wrapping_add(1);
            state.pay_as_you_go.take().map(|_| queue_event(&state))
        };
        if let Some(event) = event {
            self.publish_queue(event, &["payAsYouGo"]);
        }
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
            raw.get("payAsYouGo")
                .and_then(super::super::payment::normalize_offer),
            expected_revision,
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
        self.set_queue_state(&Value::Null, &json!([]), None, revision);
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
            let reason = if rows.len() == state.job_waiting_reasons.len()
                && state
                    .waiting_reason
                    .as_ref()
                    .is_none_or(|reason| reason.reason != "payment_pending")
            {
                if state.pay_as_you_go.is_none() {
                    return;
                }
                state.waiting_reason.clone()
            } else {
                rows.first().map(|row| row.waiting_reason.clone())
            };
            (state.queue_revision, reason, rows)
        };
        self.set_queue_state(&json!(reason), &json!(rows), None, revision);
    }

    fn set_queue_state(
        &self,
        reason: &Value,
        rows: &Value,
        mut offer: Option<PayAsYouGoOffer>,
        expected: u64,
    ) -> bool {
        let has_rows = rows.as_array().is_some_and(|rows| !rows.is_empty());
        let jobs = self.jobs();
        let event = {
            let mut state = self.inner.state.write();
            if expected != state.queue_revision {
                return false;
            }
            if state.status.is_finished() {
                offer = None;
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
            if state.waiting_reason == reason
                && state.job_waiting_reasons == rows
                && state.pay_as_you_go == offer
            {
                return false;
            }
            state.waiting_reason = reason;
            state.job_waiting_reasons = rows;
            state.pay_as_you_go = offer;
            state.queue_revision = state.queue_revision.wrapping_add(1);
            queue_event(&state)
        };
        self.publish_queue(event, &["waitingReason", "jobWaitingReasons", "payAsYouGo"]);
        true
    }

    fn publish_queue(&self, event: ProjectQueueChanged, fields: &[&str]) {
        self.notify("updated", json!(fields));
        if let Some(api) = self.inner.api.upgrade() {
            api.events.emit("queueChanged", json!(event));
        }
    }
}

fn queue_event(state: &ProjectState) -> ProjectQueueChanged {
    ProjectQueueChanged {
        project_id: state.id.clone(),
        waiting_reason: state.waiting_reason.clone(),
        job_waiting_reasons: state.job_waiting_reasons.clone(),
        pay_as_you_go: state.pay_as_you_go.clone(),
    }
}

fn matches_row(row: &JobWaitingReason, job: &Job) -> bool {
    row.img_id
        .as_deref()
        .is_some_and(|id| id.eq_ignore_ascii_case(&job.id()))
        || job.job_index() == Some(row.job_index)
}
