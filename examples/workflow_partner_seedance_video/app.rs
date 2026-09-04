mod config;
mod media;
mod request;
use crate::common::{
    auth::{close, connect, unique_app_id},
    cli::{execution_requested, explain_dry_run, require_confirmation},
    files::download_results,
};
use anyhow::Result;
use clap::Parser;
use config::{Args, Target, validate};
use futures_util::StreamExt as _;
use media::resolve;
use request::{chat, tool_arguments, workflow};
use serde_json::{Value, json};
use sogni_client::{Network, WorkflowStart};
pub async fn run() -> Result<()> {
    let args = Args::parse();
    validate(&args)?;
    if args.negative_prompt.is_some() {
        eprintln!(
            "Note: Seedance ignores negative prompts; use positive preservation constraints."
        );
    }
    let execute = execution_requested(args.execute, args.dry_run)?;
    if !execute {
        let media = resolve(None, &args).await?;
        let tools = tool_arguments(&args, &media);
        let body = match args.target() {
            Target::Chat => chat(&args, &tools),
            Target::Workflow => workflow(&args, &tools),
        };
        println!("{}", serde_json::to_string_pretty(&body)?);
        explain_dry_run();
        return Ok(());
    }
    let client = connect(unique_app_id("sogni-rust-partner-seedance"), Network::Fast).await?;
    let result=async{let media=resolve(Some(&client.projects),&args).await?;let tools=tool_arguments(&args,&media);
  if !args.no_estimate{let(width,height)=args.dimensions()?;let quote=client.projects.estimate_video_cost(&json!({"tokenType":args.token_type.as_str(),"model":args.model_id()?,"width":width,"height":height,"frames":(args.duration*24.0).round() as i64,"fps":24,"numberOfMedia":args.number})).await?;println!("Estimated Spark: {} (USD {})",quote.spark,quote.usd);}require_confirmation("Execute this paid hosted Seedance request?",args.yes)?;
  let response=match args.target(){Target::Chat=>client.chat.create_hosted_completion(&chat(&args,&tools)).await?,Target::Workflow=>{let body=workflow(&args,&tools);let value=client.workflows.start(WorkflowStart{input:body.get("input").cloned(),media_references:body.get("mediaReferences").and_then(Value::as_array).cloned(),token_type:Some(args.token_type.as_str().into()),billing_mode:Some(args.billing_mode.as_str().into()),confirm_cost:Some(true),..Default::default()}).await?;if args.watch{watch(&client,&value).await?}else{value}}};
  println!("{}",if args.json{serde_json::to_string_pretty(&response)?}else{summarize(&response)});if args.inspect_workflow{for id in workflow_ids(&response){println!("Workflow {id}: {}",serde_json::to_string_pretty(&client.workflows.get(&id).await?)?);}}
  let urls=media_urls(&response);if !urls.is_empty(){download_results(&urls,&args.output,"partner-seedance","mp4").await?;}Ok::<(),anyhow::Error>(())}.await;
    result.and(close(&client).await)
}
async fn watch(client: &sogni_client::SogniClient, value: &Value) -> Result<Value> {
    let id = workflow_id(value).ok_or_else(|| anyhow::anyhow!("workflow response omitted id"))?;
    let mut stream = client.workflows.stream_events(id, None, None).await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        println!("[{}] {}", event.id.as_deref().unwrap_or("-"), event.event);
        if event
            .data
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(sogni_client::CreativeWorkflowsApi::is_terminal_status)
        {
            break;
        }
    }
    Ok(client.workflows.get(id).await?)
}
fn workflow_id(value: &Value) -> Option<&str> {
    value
        .get("workflowId")
        .or_else(|| value.get("workflow_id"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
}
fn workflow_ids(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk(value, &mut |key, text| {
        if matches!(key, "workflowId" | "workflow_id") && !out.contains(&text.to_owned()) {
            out.push(text.to_owned())
        }
    });
    out
}
fn media_urls(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk(value, &mut |_, text| {
        if text.starts_with("https://")
            && (text.contains(".mp4") || text.contains("video"))
            && !out.contains(&text.to_owned())
        {
            out.push(text.to_owned())
        }
    });
    out
}
fn walk(value: &Value, f: &mut impl FnMut(&str, &str)) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    f(k, s)
                } else {
                    walk(v, f)
                }
            }
        }
        Value::Array(v) => {
            for item in v {
                walk(item, f)
            }
        }
        _ => {}
    }
}
fn summarize(value: &Value) -> String {
    workflow_id(value).map_or_else(
        || "Hosted execution completed; use --json for the complete response.".into(),
        |id| format!("Workflow: {id}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seedance_25_rejects_oversized_partner_dimension() {
        let args =
            Args::try_parse_from(["x", "--model", "seedance-2-5", "--width", "1920"]).unwrap();
        let error = validate(&args).unwrap_err().to_string();
        assert!(error.contains("capped at the 720p tier"));
    }
}
