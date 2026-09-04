mod config;
mod media;
mod request;

use anyhow::{Context, Result};
use clap::Parser;
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use sogni_client::{Network, ProjectRequest, WorkflowStart};

use crate::common::{
    auth::{close, connect, unique_app_id},
    cli::{execution_requested, explain_dry_run, require_confirmation},
    files::download_results,
    progress::wait_with_progress,
};
use config::{Args, Layer, validate};
use media::resolve_media;
use request::{build_creative_agent_request, build_direct_params};

pub async fn run() -> Result<()> {
    let args = Args::parse();
    validate(&args)?;
    let execute = execution_requested(args.execute, args.dry_run)?;
    let media = if execute {
        None
    } else {
        Some(resolve_media(None, &args).await?)
    };

    if !execute {
        print_request(&args, media.as_ref().expect("dry-run media"))?;
        explain_dry_run();
        return Ok(());
    }

    require_confirmation(
        "Submit this paid Seedance request? Pricing is server-authoritative.",
        args.yes,
    )?;
    let client = connect(unique_app_id("sogni-rust-seedance-r2v"), Network::Fast).await?;
    let result = async {
        let media = resolve_media(Some(&client.projects), &args).await?;
        match args.layer {
            Layer::Direct => {
                let request = ProjectRequest::from_value(build_direct_params(&args, &media))?;
                let estimate = client
                    .projects
                    .estimate_video_cost(&estimate_params(&request.params()))
                    .await
                    .context("estimate Seedance video cost")?;
                println!("Estimated Spark: {} (USD {})", estimate.spark, estimate.usd);
                let project = client.projects.create(request).await?;
                println!("Project: {}", project.id());
                let urls = wait_with_progress(&project).await?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&urls)?);
                }
                download_results(&urls, &args.output, "seedance-r2v", "mp4").await?;
            }
            Layer::CreativeAgent => {
                let payload = build_creative_agent_request(&args, &media);
                let start = WorkflowStart {
                    input: payload.get("input").cloned(),
                    token_type: Some("spark".into()),
                    billing_mode: Some("auto".into()),
                    media_references: payload
                        .get("mediaReferences")
                        .and_then(Value::as_array)
                        .cloned(),
                    confirm_cost: Some(args.yes),
                    ..WorkflowStart::default()
                };
                let workflow = client.workflows.start(start).await?;
                println!("{}", serde_json::to_string_pretty(&workflow)?);
                if args.watch {
                    watch_workflow(&client, &workflow).await?;
                }
            }
        }
        Result::<()>::Ok(())
    }
    .await;
    let close_result = close(&client).await;
    result.and(close_result)
}

fn print_request(args: &Args, media: &media::MediaUrls) -> Result<()> {
    let request = match args.layer {
        Layer::Direct => json!({"layer": "direct", "request": build_direct_params(args, media)}),
        Layer::CreativeAgent => {
            json!({"layer": "creative-agent", "request": build_creative_agent_request(args, media)})
        }
    };
    println!("{}", serde_json::to_string_pretty(&request)?);
    Ok(())
}

fn estimate_params(params: &Value) -> Value {
    json!({
        "tokenType": "spark",
        "model": params["modelId"],
        "width": params["width"],
        "height": params["height"],
        "duration": params["duration"],
        "fps": params["fps"],
        "numberOfMedia": params["numberOfMedia"],
        "referenceImageCount": params["referenceImageUrls"].as_array().map_or(0, Vec::len),
        "referenceVideoCount": params["referenceVideoUrls"].as_array().map_or(0, Vec::len),
    })
}

async fn watch_workflow(client: &sogni_client::SogniClient, workflow: &Value) -> Result<()> {
    let id = workflow
        .get("workflowId")
        .or_else(|| workflow.get("workflow_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("workflow response omitted workflowId"))?;
    let mut events = client.workflows.stream_events(id, None, None).await?;
    while let Some(event) = events.next().await {
        let event = event?;
        println!(
            "[{}] {} {}",
            event.id.as_deref().unwrap_or("-"),
            event.event,
            event.data
        );
        if event
            .data
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(sogni_client::CreativeWorkflowsApi::is_terminal_status)
        {
            break;
        }
    }
    Ok(())
}
