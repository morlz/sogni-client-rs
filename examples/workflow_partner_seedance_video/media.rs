use super::config::Args;
use anyhow::{Context, Result, bail};
use serde_json::json;
use sogni_client::{ProjectsApi, detect_content_type, new_id};
use std::path::Path;
#[derive(Default)]
pub struct Media {
    pub images: Vec<String>,
    pub videos: Vec<String>,
    pub audios: Vec<String>,
    pub end_image: Option<String>,
    pub identity_audio: Option<String>,
}
pub async fn resolve(projects: Option<&ProjectsApi>, args: &Args) -> Result<Media> {
    Ok(Media {
        images: kind(projects, &args.images, "image", "referenceImage").await?,
        videos: kind(projects, &args.videos, "video", "referenceVideo").await?,
        audios: kind(projects, &args.audios, "audio", "referenceAudio").await?,
        end_image: one(
            projects,
            args.end_image.as_deref(),
            "image",
            "referenceImageEnd",
        )
        .await?,
        identity_audio: one(
            projects,
            args.audio_identity.as_deref(),
            "audio",
            "referenceAudioIdentity",
        )
        .await?,
    })
}
async fn kind(
    projects: Option<&ProjectsApi>,
    values: &[String],
    kind: &str,
    role: &str,
) -> Result<Vec<String>> {
    let mut output = Vec::new();
    for (index, value) in values.iter().enumerate() {
        output.push(resolve_one(projects, value, kind, role, index).await?);
    }
    Ok(output)
}
async fn one(
    projects: Option<&ProjectsApi>,
    value: Option<&str>,
    kind: &str,
    role: &str,
) -> Result<Option<String>> {
    match value {
        Some(v) => Ok(Some(resolve_one(projects, v, kind, role, 0).await?)),
        None => Ok(None),
    }
}
async fn resolve_one(
    projects: Option<&ProjectsApi>,
    value: &str,
    kind: &str,
    role: &str,
    index: usize,
) -> Result<String> {
    if value.starts_with("https://") {
        return Ok(value.into());
    }
    if value.starts_with("http://") || value.starts_with("data:") {
        bail!("{kind} references must be local files or HTTPS URLs");
    }
    let Some(projects) = projects else {
        return Ok(format!("https://dry-run.invalid/{kind}/{}", index + 1));
    };
    upload(projects, Path::new(value), kind, role).await
}
async fn upload(projects: &ProjectsApi, path: &Path, kind: &str, role: &str) -> Result<String> {
    if !path.is_file() {
        bail!("{kind} input is not a file: {}", path.display());
    }
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let content = detect_content_type(Some(path), &bytes)
        .filter(|v| v.starts_with(&format!("{kind}/")))
        .ok_or_else(|| anyhow::anyhow!("unsupported {kind} input: {}", path.display()))?;
    let mut query =
        json!({"jobId":format!("seedance-partner-{}",new_id()),"type":role,"contentType":content});
    if kind == "image" {
        query["imageId"] = json!(new_id());
    }
    let url = if kind == "image" {
        projects.upload_url(&query).await?
    } else {
        projects.media_upload_url(&query).await?
    };
    reqwest::Client::new()
        .put(url)
        .header(reqwest::header::CONTENT_TYPE, &content)
        .body(bytes)
        .send()
        .await?
        .error_for_status()?;
    let url = if kind == "image" {
        projects.download_url(&query).await?
    } else {
        projects.media_download_url(&query).await?
    };
    Ok(url.to_string())
}
