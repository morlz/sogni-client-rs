use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use serde_json::{Value, json};
use sogni_client::Network;

use crate::{common, shared};

const DEFAULT_SYSTEM: &str = "You are a helpful assistant.";

#[derive(Parser)]
#[command(about = "Interactive multi-turn Sogni chat")]
struct Args {
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
    #[arg(long)]
    execute: bool,
    #[arg(long, conflicts_with = "execute")]
    dry_run: bool,
}

#[derive(Default)]
struct Stats {
    turns: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    elapsed: Duration,
    ttft: Duration,
    ttft_count: u32,
}

pub async fn run() -> Result<()> {
    let args = Args::parse();
    let mut settings = shared::ChatSettings {
        model: args.model,
        max_tokens: args.max_tokens,
        temperature: args.temperature,
        top_p: args.top_p,
        top_k: args.top_k,
        think: args.think && !args.no_think,
        token_type: args.token_type,
        billing_mode: args.billing_mode,
        ..shared::ChatSettings::default()
    };
    shared::validate_settings(&settings)?;
    if !args.execute || args.dry_run {
        println!(
            "Dry run; pass --execute to start the paid interactive session.\nModel: {}\nSystem: {}",
            settings.model, args.system
        );
        return Ok(());
    }
    let client = common::auth::connect(
        common::auth::unique_app_id("sogni-chat-multi"),
        Network::Fast,
    )
    .await?;
    shared::resolve_model_defaults(&client, &mut settings).await;
    let reporter = shared::spawn_chat_reporter(&client);
    let mut session = Session {
        system: args.system,
        history: Vec::new(),
        stats: Stats::default(),
    };
    println!("Commands: /clear, /history, /system <msg>, /think, /stats, exit");
    let outcome = conversation_loop(&client, &mut settings, &mut session, args.show_thinking).await;
    reporter.abort();
    session.print_stats();
    let close = common::auth::close(&client).await;
    outcome?;
    close
}

struct Session {
    system: String,
    history: Vec<Value>,
    stats: Stats,
}

async fn conversation_loop(
    client: &sogni_client::SogniClient,
    settings: &mut shared::ChatSettings,
    session: &mut Session,
    show_thinking: bool,
) -> Result<()> {
    loop {
        let input = shared::prompt_line("You: ")?;
        if input.is_empty() {
            continue;
        }
        if matches!(input.to_ascii_lowercase().as_str(), "exit" | "quit") {
            break;
        }
        if input.starts_with('/') && handle_command(&input, settings, session) {
            continue;
        }
        session
            .history
            .push(json!({"role": "user", "content": input}));
        let mut messages = vec![json!({"role": "system", "content": session.system})];
        messages.extend(session.history.iter().cloned());
        shared::estimate_and_print(client, settings, &messages).await;
        let request = shared::runtime::request(settings, &messages, true);
        print!("\nAssistant: ");
        let started = Instant::now();
        match shared::stream_response(client, &request, show_thinking).await {
            Ok((content, result, ttft)) => {
                session
                    .history
                    .push(json!({"role": "assistant", "content": content}));
                session.stats.turns += 1;
                session.stats.elapsed += if result.time_taken.is_finite() && result.time_taken > 0.0
                {
                    Duration::from_secs_f64(result.time_taken)
                } else {
                    started.elapsed()
                };
                if !ttft.is_zero() {
                    session.stats.ttft += ttft;
                    session.stats.ttft_count += 1;
                }
                session.stats.prompt_tokens += result
                    .usage
                    .get("prompt_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                session.stats.completion_tokens += result
                    .usage
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                println!(
                    "\n  [{} completion tokens, {:.1}s, TTFT {:.2}s]\n",
                    result
                        .usage
                        .get("completion_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    started.elapsed().as_secs_f64(),
                    ttft.as_secs_f64()
                );
            }
            Err(error) => {
                session.history.pop();
                eprintln!("\n  Error: {error}\n");
            }
        }
    }
    Ok(())
}

fn handle_command(input: &str, settings: &mut shared::ChatSettings, session: &mut Session) -> bool {
    let (command, argument) = input.split_once(' ').unwrap_or((input, ""));
    match command.to_ascii_lowercase().as_str() {
        "/clear" => {
            session.history.clear();
            println!("Conversation history cleared.\n");
        }
        "/history" => {
            println!("System: {}", session.system);
            for message in &session.history {
                let role = message.get("role").and_then(Value::as_str).unwrap_or("?");
                let content = message.get("content").and_then(Value::as_str).unwrap_or("");
                println!("{role}: {}", preview(content, 100));
            }
            println!();
        }
        "/system" => {
            if argument.trim().is_empty() {
                println!("System: {}\n", session.system);
            } else {
                session.system = argument.trim().to_owned();
                println!("System prompt updated.\n");
            }
        }
        "/think" => {
            settings.think = !settings.think;
            println!(
                "Thinking {}.\n",
                if settings.think {
                    "enabled"
                } else {
                    "disabled"
                }
            );
        }
        "/stats" => session.print_stats(),
        _ => {
            println!("Unknown command: {command}\n");
        }
    }
    true
}

impl Session {
    fn print_stats(&self) {
        if self.stats.turns == 0 {
            return;
        }
        let average_ttft = if self.stats.ttft_count == 0 {
            0.0
        } else {
            self.stats.ttft.as_secs_f64() / f64::from(self.stats.ttft_count)
        };
        println!(
            "Session: {} turns, {} tokens ({} prompt + {} completion), {:.2}s, avg TTFT {:.2}s, {} messages",
            self.stats.turns,
            self.stats.prompt_tokens + self.stats.completion_tokens,
            self.stats.prompt_tokens,
            self.stats.completion_tokens,
            self.stats.elapsed.as_secs_f64(),
            average_ttft,
            self.history.len()
        );
    }
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
