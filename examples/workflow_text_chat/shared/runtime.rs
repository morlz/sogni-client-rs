use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde_json::{Value, json};
use sogni_client::{ChatCompletion, SogniClient};

use super::ThinkingFilter;

pub const DEFAULT_MODEL: &str = "qwen3.6-35b-a3b-gguf-iq4xs";

#[derive(Clone, Debug)]
pub struct ChatSettings {
    pub model: String,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<u64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub think: bool,
    pub task_profile: String,
    pub token_type: String,
    pub billing_mode: String,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.into(),
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            frequency_penalty: None,
            presence_penalty: None,
            think: false,
            task_profile: "general".into(),
            token_type: default_token_type(),
            billing_mode: default_billing_mode(),
        }
    }
}

pub fn validate_settings(settings: &ChatSettings) -> Result<()> {
    if settings.max_tokens == Some(0) {
        bail!("--max-tokens must be positive");
    }
    if settings
        .temperature
        .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
    {
        bail!("--temperature must be between 0 and 2");
    }
    if settings
        .top_p
        .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        bail!("--top-p must be between 0 and 1");
    }
    if settings.top_k == Some(0) {
        bail!("--top-k must be positive");
    }
    for (name, value) in [
        ("--freq-penalty", settings.frequency_penalty),
        ("--pres-penalty", settings.presence_penalty),
    ] {
        if value.is_some_and(|value| !value.is_finite() || !(-2.0..=2.0).contains(&value)) {
            bail!("{name} must be between -2 and 2");
        }
    }
    if !matches!(
        settings.token_type.to_ascii_lowercase().as_str(),
        "spark" | "sogni"
    ) {
        bail!("token type must be spark or sogni");
    }
    if !matches!(
        settings.billing_mode.to_ascii_lowercase().as_str(),
        "auto" | "subscription" | "tokens"
    ) {
        bail!("billing mode must be auto, subscription, or tokens");
    }
    if settings.task_profile.trim().is_empty() {
        bail!("task profile cannot be empty");
    }
    Ok(())
}

pub fn default_token_type() -> String {
    std::env::var("SOGNI_TOKEN_TYPE").unwrap_or_else(|_| "sogni".into())
}

pub fn default_billing_mode() -> String {
    std::env::var("SOGNI_BILLING_MODE").unwrap_or_else(|_| "auto".into())
}

pub fn prompt_line(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_owned())
}

pub async fn resolve_model_defaults(client: &SogniClient, options: &mut ChatSettings) {
    let Ok(models) = client.chat.wait_for_models(Duration::from_secs(15)).await else {
        eprintln!("Warning: no LLM model catalog received; using local defaults.");
        fill_fallbacks(options, None);
        return;
    };
    print_available_models(&models, &options.model);
    fill_fallbacks(options, models.get(&options.model));
}

fn fill_fallbacks(options: &mut ChatSettings, model: Option<&Value>) {
    if options.max_tokens.is_none() {
        options.max_tokens = model
            .and_then(|value| {
                if options.think {
                    value
                        .pointer("/maxOutputTokens/max")
                        .and_then(Value::as_u64)
                } else {
                    None
                }
                .or_else(|| {
                    value
                        .pointer("/maxOutputTokens/default")
                        .and_then(Value::as_u64)
                })
            })
            .or(Some(8192));
    }
    let defaults = model.and_then(|value| {
        value.get(if options.think {
            "defaultsThinking"
        } else {
            "defaultsNonThinking"
        })
    });
    options.temperature = options
        .temperature
        .or_else(|| {
            defaults
                .and_then(|value| value.get("temperature"))
                .and_then(Value::as_f64)
        })
        .or(Some(0.7));
    options.top_p = options
        .top_p
        .or_else(|| {
            defaults
                .and_then(|value| value.get("top_p"))
                .and_then(Value::as_f64)
        })
        .or(Some(0.9));
    options.top_k = options.top_k.or_else(|| {
        defaults
            .and_then(|value| value.get("top_k"))
            .and_then(Value::as_u64)
    });
    options.frequency_penalty = options.frequency_penalty.or(Some(0.0));
    options.presence_penalty = options.presence_penalty.or_else(|| {
        defaults
            .and_then(|value| value.get("presence_penalty"))
            .and_then(Value::as_f64)
            .or(Some(0.0))
    });
}

pub fn print_available_models(models: &std::collections::HashMap<String, Value>, selected: &str) {
    println!("Available LLM models:");
    let mut ids: Vec<_> = models.keys().collect();
    ids.sort_by_key(|id| (*id != selected, *id));
    for (index, id) in ids.iter().enumerate() {
        let workers = models[*id]
            .get("workers")
            .or_else(|| models[*id].get("workerCount"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        println!("  [{}] {id} ({workers} workers)", index + 1);
    }
    if !models.contains_key(selected) {
        println!("Selected model {selected:?} is offline; the request may queue.");
    }
    println!();
}

pub fn request(settings: &ChatSettings, messages: &[Value], stream: bool) -> Value {
    json!({
        "model": settings.model,
        "messages": messages,
        "max_tokens": settings.max_tokens,
        "temperature": settings.temperature,
        "top_p": settings.top_p,
        "top_k": settings.top_k,
        "frequency_penalty": settings.frequency_penalty,
        "presence_penalty": settings.presence_penalty,
        "stream": stream,
        "tokenType": settings.token_type,
        "billingMode": settings.billing_mode,
        "think": settings.think,
        "taskProfile": settings.task_profile,
    })
}

pub async fn estimate_and_print(client: &SogniClient, settings: &ChatSettings, messages: &[Value]) {
    let estimate = client
        .chat
        .estimate_cost(&json!({
            "model": settings.model,
            "messages": messages,
            "max_tokens": settings.max_tokens,
            "tokenType": settings.token_type,
            "think": settings.think,
            "taskProfile": settings.task_profile,
        }))
        .await;
    match estimate {
        Ok(value) => println!(
            "Estimated cost: {} {} (USD {})",
            display_number(value.get("costInToken")),
            settings.token_type.to_ascii_uppercase(),
            display_number(value.get("costInUSD"))
        ),
        Err(error) => {
            eprintln!("Cost estimate unavailable ({error}); server authorization is authoritative.")
        }
    }
}

fn display_number(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_f64)
        .map(|value| format!("{value:.6}"))
        .or_else(|| value.map(ToString::to_string))
        .unwrap_or_else(|| "n/a".into())
}

pub fn spawn_chat_reporter(client: &SogniClient) -> tokio::task::JoinHandle<()> {
    let mut events = client.chat.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if event.name != "jobState" {
                continue;
            }
            let kind = event
                .data
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("state");
            let worker = event
                .data
                .get("workerName")
                .and_then(Value::as_str)
                .map(|value| format!(" [{value}]"))
                .unwrap_or_default();
            eprintln!("\nStatus: {kind}{worker}");
        }
    })
}

pub async fn stream_response(
    client: &SogniClient,
    params: &Value,
    show_thinking: bool,
) -> Result<(String, ChatCompletion, Duration)> {
    let started = Instant::now();
    let mut first_token = None;
    let mut raw = String::new();
    let mut filter = ThinkingFilter::new(show_thinking);
    let mut stream = client.chat.stream_completion(params).await?;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if !chunk.content.is_empty() {
            first_token.get_or_insert_with(Instant::now);
            raw.push_str(&chunk.content);
            filter.write(&chunk.content)?;
        }
    }
    let _visible = filter.finish()?;
    let completion = stream
        .final_result()
        .context("stream ended without a terminal completion")?;
    let ttft = first_token
        .map(|time| time.duration_since(started))
        .unwrap_or_default();
    Ok((raw, completion, ttft))
}

pub fn completion_summary(result: &ChatCompletion, elapsed: Duration, ttft: Option<Duration>) {
    println!("\n{}", "-".repeat(60));
    if let Some(worker) = &result.worker_name {
        println!("Worker:       {worker}");
    }
    if let Some(ttft) = ttft {
        println!("TTFT:         {:.2}s", ttft.as_secs_f64());
    }
    println!(
        "Time:         {:.2}s (server: {:.2}s)",
        elapsed.as_secs_f64(),
        result.time_taken
    );
    println!("Finish:       {}", result.finish_reason);
    let prompt = result.usage.get("prompt_tokens").and_then(Value::as_u64);
    let completion = result
        .usage
        .get("completion_tokens")
        .and_then(Value::as_u64);
    let total = result.usage.get("total_tokens").and_then(Value::as_u64);
    if let (Some(prompt), Some(completion), Some(total)) = (prompt, completion, total) {
        println!("Tokens:       {prompt} prompt + {completion} completion = {total} total");
        if result.time_taken > 0.0 {
            println!(
                "Speed:        {:.1} tokens/sec",
                completion as f64 / result.time_taken
            );
        }
    }
    if let Some(cost) = &result.cost {
        println!("Cost:         {cost}");
    }
}
