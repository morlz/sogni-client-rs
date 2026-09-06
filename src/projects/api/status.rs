use super::*;

impl ProjectsApi {
    /// Read owner-scoped v2 status for an active or terminal generation project.
    /// HTTP success alone does not imply completion; inspect `finished` and `status`.
    /// Compact failed/canceled records may omit job details and expire after 24 hours.
    /// The legacy terminal-result [`Self::get`] contract is unchanged.
    pub async fn get_status(&self, project_id: &str) -> Result<Value> {
        require_nonempty(project_id, "project_id")?;
        // The v2 owner-status route is case-sensitive even though UUID identity is not.
        let canonical_id = project_id.to_uppercase();
        let response = self
            .inner
            .client
            .rest
            .get(
                &format!("/v2/projects/{}", path_segment(&canonical_id)),
                None,
            )
            .await?;
        let project = response.pointer("/data/project").ok_or_else(|| {
            Error::Protocol("project status response missing data.project".into())
        })?;
        validate(project_id, project)?;
        Ok(project.clone())
    }
}

fn validate(project_id: &str, project: &Value) -> Result<()> {
    let identity_matches = project
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| id.eq_ignore_ascii_case(project_id));
    let terminal = match project.get("status").and_then(Value::as_str) {
        Some("pending" | "queued" | "processing") => false,
        Some("completed" | "failed" | "canceled") => true,
        _ => return Err(invalid()),
    };
    if !identity_matches || project.get("finished").and_then(Value::as_bool) != Some(terminal) {
        return Err(invalid());
    }
    Ok(())
}

fn invalid() -> Error {
    Error::Protocol("project status response is inconsistent".into())
}

#[cfg(test)]
mod tests;
