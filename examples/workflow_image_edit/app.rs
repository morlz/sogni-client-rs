use anyhow::Result;
use sogni_client::{AssetRole, MediaSource, Network, ProjectRequest};

use crate::{
    common::{
        auth::{close, connect, unique_app_id},
        cli::{execution_requested, explain_dry_run},
        files::slug,
        workflow::{estimate_and_confirm, print_request, require_model, run_image_project},
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
    for (index, path) in config.contexts.iter().enumerate() {
        request = request.asset(
            AssetRole::ContextImage((index + 1) as u8),
            MediaSource::Path(path.clone()),
        );
    }
    request
}

pub async fn run(args: Args) -> Result<()> {
    let config = args.resolve()?;
    let request = request(&config);
    println!(
        "{} with {} reference image(s):",
        config.model.name,
        config.contexts.len()
    );
    print_request(&request)?;
    if !execution_requested(config.execute, config.dry_run)? {
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("sogni-workflow-image-edit"), Network::Fast).await?;
    let result = async {
        require_model(&client.projects, config.model.id).await?;
        estimate_and_confirm(&client.projects, &request, config.yes).await?;
        let prefix = format!(
            "{}-{}x{}-{}-{}",
            config.model.key,
            config.width,
            config.height,
            config.seed,
            slug(&config.prompt, 30)
        );
        run_image_project(
            &client.projects,
            request,
            &config.output,
            &prefix,
            &config.output_format,
        )
        .await?;
        Ok(())
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_numbers_context_assets() {
        let config = Config {
            prompt: "edit".into(),
            contexts: vec!["one.png".into(), "two.png".into()],
            model: crate::common::models::edit_model("krea-identity-edit").unwrap(),
            width: 1024,
            height: 1024,
            batch: 1,
            seed: 9,
            guidance: 1.0,
            steps: 10,
            sampler: "euler".into(),
            scheduler: "simple".into(),
            negative: None,
            style: None,
            output: "output".into(),
            output_format: "jpg".into(),
            disable_filter: false,
            billing_mode: crate::common::cli::BillingMode::Auto,
            token_type: crate::common::cli::TokenType::Spark,
            execute: false,
            dry_run: true,
            yes: false,
        };
        let params = request(&config).params();
        assert_eq!(params["contextImage1"], true);
        assert_eq!(params["contextImage2"], true);
    }
}
