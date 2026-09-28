use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub struct GetProjectResultOptions {
    /// Used only when model metadata and stored result evidence do not identify the media.
    pub kind: Option<ResultMediaKind>,
}

#[derive(Clone, Debug, Default)]
pub struct ListRecentProjectsOptions {
    /// Oldest finish time, in milliseconds since the epoch. Defaults to 24 hours ago.
    /// History is limited to seven days; older values are clamped inside that window.
    pub since: Option<i64>,
    /// Number of renders to read, clamped to 1–100. Defaults to 50.
    pub limit: Option<u32>,
    pub app_source: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResultUrlUnavailable {
    SensitiveContent,
    UnknownMediaKind,
    DownloadUrlFailed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectResultJob {
    pub id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ResultMediaKind>,
    /// Signed URLs expire; call `get_result` again to request fresh URLs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_unavailable: Option<ResultUrlUnavailable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectResult {
    pub id: String,
    pub status: ProjectStatus,
    pub finished: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_reason: Option<WaitingReason>,
    pub jobs: Vec<ProjectResultJob>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecentProjectJob {
    pub id: String,
    pub status: String,
    pub sensitive_content_withheld: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub jobs: Vec<RecentProjectJob>,
}

impl ProjectsApi {
    /// Read one account-owned project and collect completed render URLs without
    /// submitting work or changing local tracking. Also works after socket recovery expires.
    pub async fn get_result(
        &self,
        project_id: &str,
        options: Option<GetProjectResultOptions>,
    ) -> Result<ProjectResult> {
        let session = self.inner.client.rest.request_session();
        session
            .run(async {
                let snapshot = self.get_status(project_id).await?;
                session.check()?;
                let model_id = text(snapshot.pointer("/model/id"));
                let canonical_id = snapshot["id"].as_str().expect("validated status identity");
                let project_type = snapshot.pointer("/model/type").and_then(Value::as_str);
                let mut jobs = Vec::new();
                let mut seen = HashSet::new();
                for raw in records(&snapshot, "completedWorkerJobs")
                    .chain(records(&snapshot, "workerJobs"))
                {
                    let Some(id) = result_id(raw) else {
                        continue;
                    };
                    if !seen.insert(id.to_owned()) {
                        continue;
                    }
                    let status = result_status(raw);
                    let mut job = ProjectResultJob {
                        id: id.into(),
                        status: status.clone(),
                        reason: (status != "completed")
                            .then(|| text(raw.get("reason")))
                            .flatten(),
                        kind: None,
                        url: None,
                        url_unavailable: None,
                        seed: raw.get("seedUsed").and_then(Value::as_u64),
                    };
                    if status == "completed" {
                        if withheld(raw) {
                            job.url_unavailable = Some(ResultUrlUnavailable::SensitiveContent);
                        } else if let Some(url) = text(raw.get("resultUrl")) {
                            job.url = Some(url);
                        } else {
                            let evidence = result_media_evidence(raw);
                            let kind = self
                                .result_media_kind(
                                    model_id.as_deref(),
                                    project_type,
                                    evidence.as_ref(),
                                )
                                .or(options.and_then(|options| options.kind));
                            if let Some(kind) = kind {
                                job.kind = Some(kind);
                                let content_type = (kind == ResultMediaKind::Audio)
                                    .then(|| {
                                        evidence
                                            .as_ref()
                                            .and_then(|evidence| evidence.content_type.as_deref())
                                    })
                                    .flatten();
                                match self
                                    .mint_result_url(canonical_id, id, kind, content_type)
                                    .await
                                {
                                    Ok(url) => job.url = Some(url),
                                    Err(_) => {
                                        job.url_unavailable =
                                            Some(ResultUrlUnavailable::DownloadUrlFailed)
                                    }
                                }
                                session.check()?;
                            } else {
                                job.url_unavailable = Some(ResultUrlUnavailable::UnknownMediaKind);
                            }
                        }
                    }
                    jobs.push(job);
                }
                Ok(ProjectResult {
                    id: snapshot["id"]
                        .as_str()
                        .expect("validated status identity")
                        .into(),
                    status: serde_json::from_value(snapshot["status"].clone())?,
                    finished: snapshot["finished"]
                        .as_bool()
                        .expect("validated status terminal flag"),
                    model_id,
                    waiting_reason: snapshot
                        .get("waitingReason")
                        .and_then(queue::normalize_waiting_reason),
                    jobs,
                })
            })
            .await
    }

    /// List this account's recent completed media projects from durable history,
    /// newest first. Use `get_result` to obtain their signed download URLs.
    pub async fn list_recent(
        &self,
        options: Option<ListRecentProjectsOptions>,
    ) -> Result<Vec<RecentProject>> {
        let session = self.inner.client.rest.request_session();
        session.run(async {
            if !self.inner.client.is_authenticated() { return Err(Error::InvalidInput("list_recent needs a signed-in account".into())); }
            // Fetch under this operation's session rather than retaining a second account projection.
            let me = self.inner.client.rest.get("/v1/account/me", None).await?;
            session.check()?;
            let address = me.pointer("/data/walletAddress").or_else(|| me.pointer("/data/wallet_address")).and_then(Value::as_str)
                .filter(|address| !address.is_empty()).ok_or_else(|| Error::InvalidInput("list_recent needs a signed-in account".into()))?;
            let options = options.unwrap_or_default();
            let now = Utc::now().timestamp_millis();
            let since = options.since.unwrap_or(now - 24 * 60 * 60 * 1000).max(now - (7 * 24 * 60 * 60 * 1000 - 60 * 1000));
            let query = json!({"role":"artist","address":address,"state":"completed","mediaOnly":true,"since":since,
                "limit":options.limit.unwrap_or(50).clamp(1,100),"appSource":options.app_source.filter(|source| !source.is_empty())});
            let response = self.inner.client.rest.get("/v1/jobs/list", Some(&query)).await?;
            session.check()?;
            let mut projects: Vec<RecentProject> = Vec::new();
            for raw in response.pointer("/data/jobs").and_then(Value::as_array).into_iter().flatten() {
                let Some(id) = raw.pointer("/parentRequest/id").and_then(Value::as_str).filter(|id| !id.is_empty()) else { continue; };
                let Some(job_id) = result_id(raw) else { continue; };
                let index = projects.iter().position(|project| project.id == id).unwrap_or_else(|| {
                    projects.push(RecentProject { id:id.into(), model_id:text(raw.pointer("/parentRequest/model/id")), model_name:text(raw.pointer("/parentRequest/model/name")), app_source:text(raw.pointer("/parentRequest/appSource")), finished_at:None, jobs:Vec::new() });
                    projects.len() - 1
                });
                let project = &mut projects[index];
                let finished_at = raw.get("endTime").and_then(Value::as_i64).filter(|time| *time > 0);
                project.finished_at = project.finished_at.max(finished_at);
                project.jobs.push(RecentProjectJob { id:job_id.into(), status:result_status(raw), sensitive_content_withheld:withheld(raw), finished_at });
            }
            projects.sort_by_key(|project| std::cmp::Reverse(project.finished_at.unwrap_or(0)));
            Ok(projects)
        }).await
    }
}

fn records<'a>(value: &'a Value, name: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}
fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}
fn result_id(raw: &Value) -> Option<&str> {
    raw.get("imgID")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .or_else(|| {
            raw.get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
        })
}
fn result_status(raw: &Value) -> String {
    match raw.get("status").and_then(Value::as_str) {
        Some("jobCompleted") => "completed",
        Some("jobError") if raw.get("reason").and_then(Value::as_str) == Some("artistCanceled") => {
            "canceled"
        }
        Some("jobError") => "failed",
        Some(status) if !status.is_empty() => status,
        _ => "unknown",
    }
    .into()
}
fn withheld(raw: &Value) -> bool {
    raw.get("triggeredNSFWFilter").and_then(Value::as_bool) == Some(true)
        && raw.get("nsfwDetected").and_then(Value::as_bool) != Some(true)
}
