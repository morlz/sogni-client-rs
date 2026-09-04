use anyhow::{Result, bail};
use clap::Parser;
use serde_json::{Value, json};

use crate::common;

const DIRECT_TOOLS: &[&str] = &[
    "enhance_prompt",
    "compose_script",
    "compose_lyrics",
    "compose_instrumental",
    "compose_workflow",
    "compose_workflow_template",
];

#[derive(Parser)]
#[command(about = "Execute a known synchronous hosted creative tool directly")]
struct Args {
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,
    #[arg(long, default_value = "enhance_prompt")]
    tool: String,
    #[arg(long, default_value = "spark")]
    token_type: String,
    #[arg(long, default_value = "generate_image")]
    destination_tool: String,
    #[arg(long, default_value = "")]
    destination_model: String,
    #[arg(long, default_value = "Generated Workflow Template")]
    name: String,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    execute: bool,
    #[arg(long, conflicts_with = "execute")]
    dry_run: bool,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    if !DIRECT_TOOLS.contains(&args.tool.as_str()) {
        bail!(
            "unsupported direct tool {:?}; use --help for the list",
            args.tool
        );
    }
    if !matches!(
        args.token_type.to_ascii_lowercase().as_str(),
        "auto" | "spark" | "sogni"
    ) {
        bail!("--token-type must be auto, spark, or sogni");
    }
    let missing_prompt = args.prompt.is_empty();
    let prompt = if args.prompt.is_empty() {
        "<prompt or brief>".into()
    } else {
        args.prompt.join(" ")
    };
    let request = json!({
        "tool": args.tool,
        "arguments": arguments(&args, &prompt),
        "tokenType": args.token_type,
        "appSource": "sogni-client-rs-example",
    });
    if !args.execute || args.dry_run {
        println!("Dry run; pass --execute to submit this paid request.\n{request:#}");
        return Ok(());
    }
    if missing_prompt {
        bail!("prompt or brief is required");
    }
    let credentials = common::auth::load_credentials()?;
    let client = common::auth::connect_api_key_rest_only(
        common::auth::unique_app_id("direct-creative-tool"),
        credentials,
    )
    .await?;
    let outcome = client.chat.execute_hosted_tool(&request).await;
    if let Ok(response) = &outcome {
        if args.json {
            println!("{}", serde_json::to_string_pretty(response)?);
        } else if let Some(message) = best_message(response) {
            println!("{message}");
        } else {
            println!("{}", serde_json::to_string_pretty(response)?);
        }
    }
    let close = common::auth::close(&client).await;
    outcome?;
    close
}

fn arguments(args: &Args, prompt: &str) -> Value {
    match args.tool.as_str() {
        "enhance_prompt" => json!({
            "prompt": prompt,
            "destination_tool": args.destination_tool,
            "destination_model": optional(&args.destination_model),
        }),
        "compose_script" => json!({
            "brief": prompt,
            "script_type": if args.destination_tool == "generate_video" { "video_prompt" } else { "creative_brief" },
            "destination_model": optional(&args.destination_model),
        }),
        "compose_lyrics" => json!({"prompt": prompt, "language": "unknown"}),
        "compose_instrumental" => json!({"prompt": prompt}),
        "compose_workflow" => json!({"brief": prompt}),
        "compose_workflow_template" => json!({"brief": prompt, "name": args.name}),
        _ => Value::Null,
    }
}

fn optional(value: &str) -> Value {
    if value.trim().is_empty() {
        Value::Null
    } else {
        json!(value)
    }
}

fn best_message(response: &Value) -> Option<&str> {
    [
        "/data/message",
        "/data/result/message",
        "/data/result/prompt",
        "/data/result/script",
        "/data/result/lyrics",
        "/data/result/structure",
    ]
    .into_iter()
    .find_map(|path| response.pointer(path).and_then(Value::as_str))
}
