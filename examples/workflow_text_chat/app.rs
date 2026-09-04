use std::time::Instant;

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use sogni_client::Network;

use crate::{common, shared};

const DEFAULT_SYSTEM: &str = "You are a helpful assistant.";

#[derive(Parser)]
#[command(about = "Non-streaming Sogni chat completion")]
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
    #[arg(long = "freq-penalty")]
    frequency_penalty: Option<f64>,
    #[arg(long = "pres-penalty")]
    presence_penalty: Option<f64>,
    #[arg(long, conflicts_with = "no_think")]
    think: bool,
    #[arg(long)]
    no_think: bool,
    #[arg(long, default_value = "general")]
    task_profile: String,
    #[arg(long, default_value_t = shared::default_token_type())]
    token_type: String,
    #[arg(long, default_value_t = shared::default_billing_mode())]
    billing_mode: String,
    /// Perform the paid request; otherwise print a credential-free dry run.
    #[arg(long)]
    execute: bool,
    #[arg(long, conflicts_with = "execute")]
    dry_run: bool,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let mut prompt = args.prompt.join(" ").trim().to_owned();
    let mut settings = shared::ChatSettings {
        model: args.model,
        max_tokens: args.max_tokens,
        temperature: args.temperature,
        top_p: args.top_p,
        top_k: args.top_k,
        frequency_penalty: args.frequency_penalty,
        presence_penalty: args.presence_penalty,
        think: args.think && !args.no_think,
        task_profile: args.task_profile,
        token_type: args.token_type,
        billing_mode: args.billing_mode,
    };
    shared::validate_settings(&settings)?;
    if !args.execute || args.dry_run {
        if prompt.is_empty() {
            prompt = "<interactive prompt>".into();
        }
        let messages = messages(&args.system, &prompt);
        println!(
            "Dry run; pass --execute to submit this paid request.\n{}",
            serde_json::to_string_pretty(&shared::runtime::request(&settings, &messages, false))?
        );
        return Ok(());
    }
    if prompt.is_empty() {
        prompt = shared::prompt_line("You: ")?;
    }
    if prompt.is_empty() {
        bail!("prompt is required");
    }

    let client =
        common::auth::connect(common::auth::unique_app_id("sogni-chat"), Network::Fast).await?;
    shared::resolve_model_defaults(&client, &mut settings).await;
    let messages = messages(&args.system, &prompt);
    shared::estimate_and_print(&client, &settings, &messages).await;
    let reporter = shared::spawn_chat_reporter(&client);
    let started = Instant::now();
    let outcome = client
        .chat
        .create_completion(&shared::runtime::request(&settings, &messages, false))
        .await;
    reporter.abort();
    match &outcome {
        Ok(result) => {
            println!("\nAssistant:\n\n{}", result.content);
            shared::completion_summary(result, started.elapsed(), None);
        }
        Err(error) => eprintln!("Chat completion failed: {error}"),
    }
    let close = common::auth::close(&client).await;
    outcome?;
    close
}

fn messages(system: &str, prompt: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.push(json!({"role": "user", "content": prompt}));
    messages
}
