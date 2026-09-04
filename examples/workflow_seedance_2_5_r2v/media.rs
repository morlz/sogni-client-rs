use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::json;
use sogni_client::{ProjectsApi, detect_content_type, new_id};

use super::config::Args;

#[derive(Debug, Default)]
pub struct MediaUrls {
    pub images: Vec<String>,
    pub videos: Vec<String>,
    pub audios: Vec<String>,
}

pub async fn resolve_media(projects: Option<&ProjectsApi>, args: &Args) -> Result<MediaUrls> {
    Ok(MediaUrls {
        images: resolve_kind(projects, &args.images, "image").await?,
        videos: resolve_kind(projects, &args.videos, "video").await?,
        audios: resolve_kind(projects, &args.audios, "audio").await?,
    })
}

async fn resolve_kind(
    projects: Option<&ProjectsApi>,
    inputs: &[String],
    kind: &str,
) -> Result<Vec<String>> {
    let mut urls = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        // Workflows require durable HTTPS media. Live local paths use the signed
        // upload flow; dry runs preserve slot order without reading the files.
        if input.starts_with("https://") {
            urls.push(input.clone());
        } else if input.starts_with("http://") || input.starts_with("data:") {
            bail!("{kind} references must be local files or HTTPS URLs");
        } else if let Some(projects) = projects {
            urls.push(upload_local(projects, Path::new(input), kind).await?);
        } else {
            let name = Path::new(input)
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("media");
            urls.push(format!(
                "https://dry-run.invalid/{kind}/{}-{name}",
                index + 1
            ));
        }
    }
    Ok(urls)
}

async fn upload_local(projects: &ProjectsApi, path: &Path, kind: &str) -> Result<String> {
    if !path.is_file() {
        bail!("{kind} input is not a file: {}", path.display());
    }
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let content_type = detect_content_type(Some(path), &bytes)
        .ok_or_else(|| anyhow::anyhow!("unsupported {kind} file: {}", path.display()))?;
    if !content_type.starts_with(&format!("{kind}/")) {
        bail!("unsupported {kind} file: {}", path.display());
    }
    let job_id = format!("seedance-r2v-example-{}", new_id());
    let role = format!("reference{}", uppercase_first(kind));
    let mut query = json!({"jobId": job_id, "type": role, "contentType": content_type});
    if kind == "image" {
        query["imageId"] = json!(new_id());
    }
    let upload = if kind == "image" {
        projects.upload_url(&query).await?
    } else {
        projects.media_upload_url(&query).await?
    };
    reqwest::Client::new()
        .put(upload)
        .header(reqwest::header::CONTENT_TYPE, &content_type)
        .body(bytes)
        .send()
        .await
        .context("upload reference media")?
        .error_for_status()
        .context("upload reference media")?;
    let download = if kind == "image" {
        projects.download_url(&query).await?
    } else {
        projects.media_download_url(&query).await?
    };
    Ok(download.to_string())
}

fn uppercase_first(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}
