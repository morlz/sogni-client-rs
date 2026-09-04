use std::time::Instant;

use anyhow::Result;
use serde_json::{Value, json};
use sogni_client::SogniClient;

use crate::{
    response,
    session::{Session, Turn},
};

pub fn request(session: &Session, user_text: &str) -> Value {
    let mut messages = Vec::with_capacity(session.messages.len() + 2);
    messages.push(json!({"role": "system", "content": session.system_prompt()}));
    messages.extend(session.messages.iter().cloned());
    messages.push(json!({"role": "user", "content": user_text}));
    json!({
        "model": session.options.model,
        "messages": messages,
        "max_tokens": session.options.max_tokens,
        "temperature": session.options.temperature,
        "top_p": session.options.top_p,
        "token_type": session.options.token_type,
        "billingMode": session.options.billing_mode,
        "sogni_tools": session.options.tools_mode.value(),
        "sogni_tool_execution": session.options.execute_tools,
        "task_profile": "reasoning",
        "chat_template_kwargs": {"enable_thinking": session.options.think},
        "app_source": session.options.app_source,
        "stream": false
    })
}

pub async fn run_turn(client: &SogniClient, session: &mut Session, user_text: &str) -> Result<()> {
    let request = request(session, user_text);
    println!(
        "\nSending turn to Sogni ({}, billing={})...",
        session.options.tools_mode.label(),
        session.options.billing_mode
    );
    let started = Instant::now();
    let response = client.chat.create_hosted_completion(&request).await?;
    let elapsed = started.elapsed().as_secs_f64();
    session.last_response = Some(response.clone());
    session.turns.push(Turn {
        user_text: user_text.to_owned(),
        response: response.clone(),
        elapsed_seconds: elapsed,
    });
    if session.options.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        response::print(&response, elapsed);
    }
    // Mutate conversation history only after a successful hosted response. This
    // leaves retries free of a dangling user turn while retaining tool summaries.
    session
        .messages
        .push(json!({"role": "user", "content": user_text}));
    session
        .messages
        .push(json!({"role": "assistant", "content": response::assistant_history(&response)}));
    session.trim_history();
    Ok(())
}

pub async fn print_subscription(client: &SogniClient) {
    match client.account.get_subscription_status().await {
        Ok(value) => {
            let status = value.get("subscription").unwrap_or(&value);
            println!("\nSubscription:");
            println!(
                "  Active: {}",
                if status.get("active").and_then(Value::as_bool) == Some(true) {
                    "yes"
                } else {
                    "no"
                }
            );
            println!(
                "  Status: {}",
                status
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            );
            if let Some(tier) = status.get("tier").and_then(Value::as_str) {
                println!("  Tier:   {tier}")
            }
            if let Some(until) = status
                .get("currentPeriodEnd")
                .or_else(|| status.get("periodEnd"))
            {
                println!("  Until:  {until}")
            }
            println!();
        }
        Err(error) => println!("Could not fetch subscription status: {error}"),
    }
}

pub fn is_strict_subscription_error(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}");
    message.contains("402")
        && message
            .to_ascii_lowercase()
            .contains("unlimited billing is not available")
}
