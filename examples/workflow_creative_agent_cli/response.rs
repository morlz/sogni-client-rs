use std::collections::HashSet;

use serde_json::Value;

use crate::session::Session;

pub fn data(response: &Value) -> &Value {
    response.get("data").unwrap_or(response)
}

pub fn message(response: &Value) -> &Value {
    data(response)
        .pointer("/choices/0/message")
        .or_else(|| data(response).pointer("/choices/0/delta"))
        .unwrap_or(&Value::Null)
}

pub fn tool_results(response: &Value) -> &[Value] {
    data(response)
        .get("sogni_tool_results")
        .or_else(|| data(response).get("sogniToolResults"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub fn workflows(response: &Value) -> &[Value] {
    data(response)
        .get("creative_workflows")
        .or_else(|| data(response).get("creativeWorkflows"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub fn print(response: &Value, elapsed_seconds: f64) {
    let message = message(response);
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    println!(
        "\nAssistant:\n{}",
        if content.is_empty() {
            "(No text response returned.)"
        } else {
            content
        }
    );
    if let Some(calls) = message
        .get("tool_calls")
        .or_else(|| message.get("toolCalls"))
        .and_then(Value::as_array)
        .filter(|calls| !calls.is_empty())
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
    if !tool_results(response).is_empty() {
        println!("\nSogni tool results:");
        for (index, result) in tool_results(response).iter().enumerate() {
            println!("  - {}", summarize_tool_result(result, index));
        }
    }
    if !workflows(response).is_empty() {
        println!("\nCreative workflows:");
        for workflow in workflows(response) {
            let id = first_string(workflow, &["workflowId", "id"]).unwrap_or("workflow");
            let status = first_string(workflow, &["status"]).unwrap_or("submitted");
            println!("  - {id}: {status}");
            if let Some(url) = first_string(workflow, &["url"]) {
                println!("    {url}")
            }
        }
    }
    if let Some(usage) = data(response).get("usage") {
        let prompt = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total = usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(prompt + completion);
        println!("\nUsage: {prompt} prompt + {completion} completion = {total} tokens");
    }
    if let Some(cost) = data(response).get("cost").filter(|value| !value.is_null()) {
        println!("Cost:  {cost}");
    }
    println!("Time:  {elapsed_seconds:.2}s");
}

pub fn assistant_history(response: &Value) -> String {
    let mut blocks = Vec::new();
    if let Some(content) = message(response)
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        blocks.push(content.to_owned());
    }
    if !tool_results(response).is_empty() {
        blocks.push(format!(
            "Sogni tool results for continuity:\n{}",
            truncate(
                &serde_json::to_string_pretty(tool_results(response)).unwrap_or_default(),
                6000
            )
        ));
    }
    if !workflows(response).is_empty() {
        blocks.push(format!(
            "Creative workflow references:\n{}",
            truncate(
                &serde_json::to_string_pretty(workflows(response)).unwrap_or_default(),
                2000
            )
        ));
    }
    if blocks.is_empty() {
        "[Sogni returned no assistant text.]".into()
    } else {
        blocks.join("\n\n")
    }
}

pub fn print_history(session: &Session) {
    if session.messages.is_empty() {
        println!("History is empty.");
        return;
    }
    println!("\nHistory ({} messages kept):", session.messages.len());
    for (index, message) in session.messages.iter().enumerate() {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let content = message.get("content").and_then(Value::as_str).unwrap_or("");
        println!("  {}. {role}: {}", index + 1, compact(content, 140));
    }
    println!();
}

pub fn print_last(session: &Session, raw: bool) {
    let Some(response) = &session.last_response else {
        println!("No hosted response yet.");
        return;
    };
    if raw {
        println!(
            "{}",
            serde_json::to_string_pretty(response).unwrap_or_else(|_| response.to_string())
        );
    } else {
        let elapsed = session
            .turns
            .last()
            .map(|turn| turn.elapsed_seconds)
            .unwrap_or(0.0);
        print(response, elapsed);
    }
}

fn summarize_tool_result(result: &Value, index: usize) -> String {
    let tool = first_string(
        result,
        &[
            "tool",
            "toolName",
            "tool_name",
            "name",
            "type",
            "media_type",
        ],
    )
    .map(ToOwned::to_owned)
    .unwrap_or_else(|| format!("tool_{}", index + 1));
    let failed = result.get("success").and_then(Value::as_bool) == Some(false)
        || result.get("ok").and_then(Value::as_bool) == Some(false)
        || result.get("error").is_some_and(|value| !value.is_null());
    let mut parts = vec![format!("{tool}: {}", if failed { "failed" } else { "ok" })];
    if let Some(value) = first_string(
        result,
        &[
            "message",
            "summary",
            "prompt",
            "script",
            "lyrics",
            "structure",
            "url",
            "local_file",
            "workflowId",
            "id",
        ],
    ) {
        parts.push(compact(value, 180));
    }
    let mut urls = Vec::new();
    collect_urls(result, &mut urls, &mut HashSet::new(), 0);
    if !urls.is_empty() {
        parts.push(
            urls.into_iter()
                .take(2)
                .map(|url| compact(&url, 120))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    parts.join(" - ")
}

fn first_string<'a>(record: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        record
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn collect_urls(value: &Value, urls: &mut Vec<String>, seen: &mut HashSet<String>, depth: usize) {
    if depth > 5 || urls.len() >= 10 {
        return;
    }
    match value {
        Value::String(value)
            if (value.starts_with("http://") || value.starts_with("https://"))
                && seen.insert(value.clone()) =>
        {
            urls.push(value.clone())
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect_urls(value, urls, seen, depth + 1)),
        Value::Object(values) => values
            .values()
            .for_each(|value| collect_urls(value, urls, seen, depth + 1)),
        _ => {}
    }
}

pub fn compact(value: &str, maximum: usize) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&value, maximum).replace('\n', " ")
}

pub fn truncate(value: &str, maximum: usize) -> String {
    if value.chars().count() <= maximum {
        return value.to_owned();
    }
    let keep = maximum.saturating_sub(30);
    format!(
        "{}\n[Truncated to {maximum} characters.]",
        value.chars().take(keep).collect::<String>()
    )
}

#[cfg(test)]
fn tool_result_fixture() -> Value {
    serde_json::json!({"tool": "generate_image", "success": true})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_tool_results_for_continuity() {
        let response = serde_json::json!({"data": {"choices": [{"message": {"content": "Done"}}], "sogni_tool_results": [tool_result_fixture()]}});
        let history = assistant_history(&response);
        assert!(history.contains("Done"));
        assert!(history.contains("generate_image"));
    }
}
