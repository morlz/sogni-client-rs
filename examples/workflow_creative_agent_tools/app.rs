use anyhow::{Result, bail};
use clap::Parser;
use serde_json::{Value, json};

use crate::{common, shared};

#[derive(Parser)]
#[command(about = "Hosted Sogni creative-tool injection")]
struct Args {
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,
    #[arg(long, default_value = shared::runtime::DEFAULT_MODEL)]
    model: String,
    #[arg(long, default_value = "creative-tools")]
    tools: String,
    /// Inject tools but disable their server-side execution.
    #[arg(long)]
    no_execute: bool,
    #[arg(long, default_value = "spark")]
    token_type: String,
    #[arg(long, default_value_t = shared::default_billing_mode())]
    billing_mode: String,
    #[arg(long)]
    json: bool,
    /// Submit the paid network request. Without this, only the body is printed.
    #[arg(long)]
    submit: bool,
    #[arg(long, conflicts_with = "submit")]
    dry_run: bool,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    if !matches!(
        args.token_type.to_ascii_lowercase().as_str(),
        "spark" | "sogni"
    ) {
        bail!("--token-type must be spark or sogni");
    }
    if !matches!(
        args.billing_mode.to_ascii_lowercase().as_str(),
        "auto" | "subscription" | "tokens"
    ) {
        bail!("--billing-mode must be auto, subscription, or tokens");
    }
    if args.model.trim().is_empty() {
        bail!("--model cannot be empty");
    }
    let missing_prompt = args.prompt.is_empty();
    let prompt = if args.prompt.is_empty() {
        "<interactive creative request>".into()
    } else {
        args.prompt.join(" ")
    };
    // These controls select server-owned tool injection/execution. No returned
    // hosted tool name is dispatched as local code by this example.
    let request = json!({
        "model": args.model,
        "messages": [
            {"role": "system", "content": "You are a concise creative production assistant. Use Sogni creative tools when they help produce concrete media."},
            {"role": "user", "content": prompt},
        ],
        "temperature": 0.4,
        "max_tokens": 1600,
        "token_type": args.token_type,
        "billingMode": args.billing_mode,
        "sogni_tools": normalize_tools(&args.tools),
        "sogni_tool_execution": !args.no_execute,
    });
    if !args.submit || args.dry_run {
        println!("Dry run; pass --submit to execute this paid hosted request.\n{request:#}");
        return Ok(());
    }
    if missing_prompt {
        bail!("a prompt is required when --submit is used");
    }
    // Hosted tool injection is API-key-only and does not need a socket session.
    let credentials = common::auth::load_credentials()?;
    let client = common::auth::connect_api_key_rest_only(
        common::auth::unique_app_id("sogni-creative-tools"),
        credentials,
    )
    .await?;
    let outcome = client.chat.create_hosted_completion(&request).await;
    if let Ok(response) = &outcome {
        if args.json {
            println!("{}", serde_json::to_string_pretty(response)?);
        } else {
            print_response(response);
        }
    }
    let close = common::auth::close(&client).await;
    outcome?;
    close
}

fn normalize_tools(value: &str) -> Value {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "false" => json!(false),
        "hosted" | "true" => json!(true),
        "rich" | "creative-tools" => json!("creative-tools"),
        "creative-agent" => json!("creative-agent"),
        _ => json!(value),
    }
}

fn print_response(response: &Value) {
    let data = response.get("data").unwrap_or(response);
    let message = data
        .pointer("/choices/0/message")
        .or_else(|| data.pointer("/choices/0/delta"))
        .unwrap_or(&Value::Null);
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        println!("{content}");
    }
    if let Some(calls) = message
        .get("tool_calls")
        .or_else(|| message.get("toolCalls"))
        .and_then(Value::as_array)
    {
        println!("\nTool calls:");
        for call in calls {
            println!(
                "  - {}",
                call.pointer("/function/name")
                    .or_else(|| call.get("name"))
                    .or_else(|| call.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("tool_call")
            );
        }
    }
    let workflows = data
        .get("creative_workflows")
        .or_else(|| data.get("creativeWorkflows"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !workflows.is_empty() {
        println!("\nCreative workflows:");
        for workflow in workflows {
            println!(
                "  - {}: {}",
                workflow
                    .get("workflowId")
                    .or_else(|| workflow.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                workflow
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("submitted")
            );
        }
    }
}
