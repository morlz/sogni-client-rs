use std::time::Duration;

use anyhow::{Result, bail};

use crate::{
    common::{
        auth::{close, connect, unique_app_id},
        cli::{execution_requested, explain_dry_run, require_confirmation},
        workflow::print_request,
    },
    config::{Args, tier},
};

pub async fn run(args: Args) -> Result<()> {
    let config = args.resolve()?;
    let generations = crate::report::print_configuration(&config);
    if !execution_requested(config.execute, config.dry_run)? {
        for request in crate::run::dry_run_requests(&config) {
            print_request(&request)?;
        }
        explain_dry_run();
        return Ok(());
    }
    require_confirmation(
        &format!("Run {generations} paid image generations?"),
        config.yes,
    )?;
    let client = connect(unique_app_id("sogni-benchmark"), config.network.into()).await?;
    let result = async {
        let available = client
            .projects
            .wait_for_models(Duration::from_secs(20))
            .await?;
        let mut results = Vec::new();
        for model in &config.models {
            if !available
                .iter()
                .any(|item| item.get("id").and_then(serde_json::Value::as_str) == Some(model.id))
            {
                eprintln!("Warning: {} is unavailable; skipping it", model.id);
                continue;
            }
            let tier = tier(model.id)
                .ok_or_else(|| anyhow::anyhow!("{} has no benchmark tier", model.id))?;
            results.push(crate::run::model(&client.projects, &config, *model, tier).await);
        }
        if results.is_empty() {
            bail!("none of the selected models are currently available");
        }
        crate::report::print_results(&results);
        crate::report::save(&config, &results)
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}
