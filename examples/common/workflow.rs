use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sogni_client::{CostEstimate, Project, ProjectRequest, ProjectsApi};

use super::{cli::confirm_estimate, files::download_results, progress::wait_with_progress};

pub fn print_request(request: &ProjectRequest) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&request.params())?);
    Ok(())
}

pub async fn require_model(projects: &ProjectsApi, model_id: &str) -> Result<Value> {
    projects
        .wait_for_models(Duration::from_secs(15))
        .await
        .context("wait for available models")?
        .into_iter()
        .find(|model| model.get("id").and_then(Value::as_str) == Some(model_id))
        .ok_or_else(|| anyhow::anyhow!("model {model_id} is not currently available"))
}

/// Translate a project request into the public image-estimator contract.
pub async fn estimate_image(
    projects: &ProjectsApi,
    request: &ProjectRequest,
) -> Result<CostEstimate> {
    let params = request.params();
    let context_images = (1..=16)
        .filter(|index| {
            params
                .get(format!("contextImage{index}"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .map(|_| Value::Bool(true))
        .collect::<Vec<_>>();
    let model = params
        .get("modelId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("request has no modelId"))?;
    let size_preset = params
        .get("sizePreset")
        .filter(|value| value.as_str() != Some("custom"))
        .cloned();
    let estimate = json!({
        "tokenType": params.get("tokenType").and_then(Value::as_str).unwrap_or("spark"),
        "network": params.get("network").and_then(Value::as_str).unwrap_or("fast"),
        "model": model,
        "imageCount": params.get("numberOfMedia").and_then(Value::as_u64).unwrap_or(1),
        "stepCount": params.get("steps").and_then(Value::as_u64).unwrap_or(1),
        "previewCount": params.get("numberOfPreviews").and_then(Value::as_u64).unwrap_or(0),
        "cnEnabled": params.pointer("/controlNet/image").is_some(),
        "startingImageStrength": params.get("startingImageStrength"),
        "sizePreset": size_preset,
        "width": params.get("width"),
        "height": params.get("height"),
        "guidance": params.get("guidance"),
        "sampler": params.get("sampler"),
        "contextImages": params
            .get("contextImages")
            .cloned()
            .unwrap_or(Value::Array(context_images)),
        "gptImageQuality": params.get("gptImageQuality"),
        "outputFormat": params.get("outputFormat"),
    });
    projects.estimate_cost(&estimate).await.map_err(Into::into)
}

pub async fn estimate_and_confirm(
    projects: &ProjectsApi,
    request: &ProjectRequest,
    assume_yes: bool,
) -> Result<CostEstimate> {
    let estimate = estimate_image(projects, request).await?;
    confirm_estimate(&estimate, assume_yes)?;
    Ok(estimate)
}

pub async fn run_image_project(
    projects: &ProjectsApi,
    request: ProjectRequest,
    output_dir: &Path,
    prefix: &str,
    extension: &str,
) -> Result<(Project, Vec<std::path::PathBuf>)> {
    let project = projects.create(request).await?;
    println!("Submitted project {}", project.id());
    let urls = wait_with_progress(&project).await?;
    if urls.is_empty() {
        bail!("project completed without downloadable results");
    }
    let paths = download_results(&urls, output_dir, prefix, extension).await?;
    Ok((project, paths))
}
