use anyhow::Result;
use serde_json::json;
use sogni_client::{AssetRole, MediaSource, Network, ProjectRequest};

use crate::{
    common::{
        auth::{close, connect, unique_app_id},
        cli::{execution_requested, explain_dry_run},
        files::download_results,
        progress::{spawn_preview_downloader, wait_with_progress},
        workflow::{estimate_and_confirm, print_request, require_model},
    },
    config::{Args, Config},
};

fn request(config: &Config) -> ProjectRequest {
    let mut request = ProjectRequest::image(config.model.id, &config.prompt)
        .network(Network::Fast)
        .number_of_media(config.batch)
        .dimensions(config.width, config.height)
        .steps(config.steps)
        .guidance(config.guidance)
        .param("seed", config.seed)
        .param("numberOfPreviews", config.previews)
        .param("sampler", config.sampler.as_str())
        .param("scheduler", config.scheduler.as_str())
        .param("sizePreset", "custom")
        .param("outputFormat", config.output_format.as_str())
        .param("tokenType", config.token_type.as_str())
        .param("billingMode", config.billing_mode.as_str())
        .param("disableNSFWFilter", config.disable_filter);
    if let Some(negative) = &config.negative {
        request = request.param("negativePrompt", negative.as_str());
    }
    if let Some(style) = &config.style {
        request = request.param("stylePrompt", style.as_str());
    }
    if let Some(image) = &config.starting_image {
        request = request
            .asset(AssetRole::StartingImage, MediaSource::Path(image.clone()))
            .param("startingImageStrength", config.strength);
    }
    if let Some(lora) = &config.style_lora {
        // LoRA ids and strengths are parallel positional arrays on the wire.
        request = request
            .param("loras", json!([lora]))
            .param("loraStrengths", json!([config.lora_strength]));
    }
    request
}

pub async fn run(args: Args) -> Result<()> {
    let config = args.resolve()?;
    let request = request(&config);
    println!("Text-to-image configuration:");
    print_request(&request)?;
    if !execution_requested(config.execute, config.dry_run)? {
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("sogni-workflow-t2i"), Network::Fast).await?;
    let result = async {
        require_model(&client.projects, config.model.id).await?;
        estimate_and_confirm(&client.projects, &request, config.yes).await?;
        let prefix = config.filename_prefix();
        let project = client.projects.create(request).await?;
        // Preview polling is observational; final media comes from project completion.
        let previews = (config.previews > 0).then(|| {
            spawn_preview_downloader(
                project.clone(),
                config.output.clone(),
                prefix.clone(),
                config.output_format.clone(),
            )
        });
        let urls = wait_with_progress(&project).await?;
        if let Some(previews) = previews {
            let _ = previews.await;
        }
        download_results(&urls, &config.output, &prefix, &config.output_format).await?;
        Ok(())
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;

    #[test]
    fn builds_starting_image_and_lora_markers() {
        let fixture = std::path::PathBuf::from("examples/test-assets/placeholder.jpg");
        if !fixture.exists() {
            return;
        }
        let config = Args::parse_from([
            "test",
            "prompt",
            "--no-interactive",
            "--starting-image",
            fixture.to_str().unwrap(),
            "--style-lora",
            "style-id",
        ])
        .resolve_for_test()
        .unwrap();
        let params = request(&config).params();
        assert_eq!(params["startingImage"], true);
        assert_eq!(params["loras"], json!(["style-id"]));
    }
}
