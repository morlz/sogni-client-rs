use super::*;

#[cfg(test)]
mod tests;

impl ProjectsApi {
    /// Retire local state only; a session boundary never cancels remote work.
    pub(in crate::projects) fn clear_previous_sessions(&self) {
        let current = self.inner.client.auth_session();
        let retired = {
            let mut projects = self.inner.projects.write();
            let retired = projects
                .values()
                .filter(|project| project.auth_session() != current)
                .cloned()
                .collect::<Vec<_>>();
            projects.retain(|_, project| project.auth_session() == current);
            retired
        };
        let retained = self
            .inner
            .projects
            .read()
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        let mut submission = self.inner.submission.lock();
        if submission.handled_session != Some(current) {
            submission.handled_session = Some(current);
            submission.invalidate_rechecks();
            submission.unadmitted.retain(|id, _| retained.contains(id));
            submission
                .sent_on
                .retain(|_, (_, session)| *session == current);
            submission.awaiting.retain(|id| retained.contains(id));
            submission
                .submitted_at
                .retain(|id, _| retained.contains(id));
            self.inner.recovered_completed_ids.write().clear();
        }
        drop(submission);
        for project in retired {
            project.end_session();
        }
    }
}
