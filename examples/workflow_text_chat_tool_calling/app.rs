mod calculate;
mod tools;
mod units;
mod world;

use std::time::Instant;

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use sogni_client::{Network, SogniClient};

use crate::{common, shared};

const DEFAULT_SYSTEM: &str = "You are a helpful assistant with access to tools. Use tools when they improve accuracy. Always answer naturally after receiving tool results.";

#[derive(Parser)]
#[command(about = "Sogni custom tool-calling loop: weather, time, units, and math")]
struct Args {
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,
    #[arg(long, default_value = shared::runtime::DEFAULT_MODEL)]
    model: String,
    #[arg(long)]
    max_tokens: Option<u64>,
    #[arg(long)]
    temperature: Option<f64>,
    #[arg(long)]
    top_p: Option<f64>,
    #[arg(long)]
    top_k: Option<u64>,
    #[arg(long, default_value = DEFAULT_SYSTEM)]
    system: String,
    #[arg(long, conflicts_with = "no_think")]
    think: bool,
    #[arg(long)]
    no_think: bool,
    #[arg(long)]
    show_thinking: bool,
    #[arg(long, default_value_t = shared::default_token_type())]
    token_type: String,
    #[arg(long, default_value_t = shared::default_billing_mode())]
    billing_mode: String,
    #[arg(long, default_value_t = 5)]
    max_rounds: usize,
    #[arg(long)]
    execute: bool,
    #[arg(long, conflicts_with = "execute")]
    dry_run: bool,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let mut prompt = args.prompt.join(" ").trim().to_owned();
    let schemas = tools::schemas();
    let mut settings = shared::ChatSettings {
        model: args.model,
        max_tokens: args.max_tokens,
        temperature: args.temperature,
        top_p: args.top_p,
        top_k: args.top_k,
        think: args.think && !args.no_think,
        task_profile: "reasoning".into(),
        token_type: args.token_type,
        billing_mode: args.billing_mode,
        ..shared::ChatSettings::default()
    };
    shared::validate_settings(&settings)?;
    if !args.execute || args.dry_run {
        if prompt.is_empty() {
            prompt = "What's the weather in Austin and what is 72 F in Celsius?".into();
        }
        let messages = vec![
            json!({"role": "system", "content": args.system}),
            json!({"role": "user", "content": prompt}),
        ];
        let mut request = shared::runtime::request(&settings, &messages, true);
        request["tools"] = Value::Array(schemas);
        request["tool_choice"] = json!("auto");
        println!("Dry run; pass --execute to submit.\n{request:#}");
        return Ok(());
    }
    if prompt.is_empty() {
        prompt = shared::prompt_line("You: ")?;
    }
    if prompt.is_empty() {
        bail!("prompt is required");
    }
    if args.max_rounds == 0 {
        bail!("--max-rounds must be at least one");
    }
    let client = common::auth::connect(
        common::auth::unique_app_id("sogni-tool-calling"),
        Network::Fast,
    )
    .await?;
    shared::resolve_model_defaults(&client, &mut settings).await;
    let messages = vec![
        json!({"role": "system", "content": args.system}),
        json!({"role": "user", "content": prompt}),
    ];
    shared::estimate_and_print(&client, &settings, &messages).await;
    let reporter = shared::spawn_chat_reporter(&client);
    let outcome = run_tool_loop(
        &client,
        &settings,
        messages,
        schemas,
        args.show_thinking,
        args.max_rounds.min(20),
    )
    .await;
    reporter.abort();
    let close = common::auth::close(&client).await;
    outcome?;
    close
}

async fn run_tool_loop(
    client: &SogniClient,
    settings: &shared::ChatSettings,
    mut messages: Vec<Value>,
    schemas: Vec<Value>,
    show_thinking: bool,
    max_rounds: usize,
) -> Result<()> {
    let total_started = Instant::now();
    for round in 1..=max_rounds {
        println!("\n{}\nRound {round}", "-".repeat(60));
        let mut request = shared::runtime::request(settings, &messages, true);
        request["tools"] = Value::Array(schemas.clone());
        request["tool_choice"] = json!("auto");
        print!("Assistant: ");
        let round_started = Instant::now();
        let (content, result, ttft) =
            shared::stream_response(client, &request, show_thinking).await?;
        println!();
        shared::completion_summary(&result, round_started.elapsed(), Some(ttft));
        if result.tool_calls.is_empty() {
            println!(
                "Total elapsed: {:.2}s",
                total_started.elapsed().as_secs_f64()
            );
            return Ok(());
        }
        messages.push(json!({
            "role": "assistant",
            "content": if content.is_empty() { Value::Null } else { json!(content) },
            "tool_calls": result.tool_calls,
        }));
        println!("Executing {} tool call(s):", result.tool_calls.len());
        for call in &result.tool_calls {
            let output = tools::execute(call).await;
            println!("  {} -> {}", call.function.name, preview(&output, 160));
            messages.push(json!({
                "role": "tool",
                "content": output,
                "tool_call_id": call.id,
                "name": call.function.name,
            }));
        }
    }
    bail!("model continued requesting tools after {max_rounds} rounds")
}

fn preview(value: &str, maximum: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(maximum).collect();
    if chars.next().is_some() {
        format!("{prefix}...")
    } else {
        prefix
    }
}
