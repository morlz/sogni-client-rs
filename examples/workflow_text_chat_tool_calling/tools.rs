use serde_json::{Value, json};
use sogni_client::ChatToolCall;

use super::{calculate, units, world};

pub fn schemas() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get live weather for a city using wttr.in.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {"type": "string", "description": "City, optionally state/country"},
                        "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]}
                    },
                    "required": ["location"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_time",
                "description": "Get current time for an IANA timezone or major city.",
                "parameters": {
                    "type": "object",
                    "properties": {"timezone": {"type": "string"}},
                    "required": ["timezone"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "convert_units",
                "description": "Convert temperature, distance, weight, or speed units.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "value": {"type": "number"},
                        "from_unit": {"type": "string"},
                        "to_unit": {"type": "string"}
                    },
                    "required": ["value", "from_unit", "to_unit"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "calculate",
                "description": "Safely evaluate arithmetic with +, -, *, /, %, ^, parentheses, sqrt, abs, min, and max.",
                "parameters": {
                    "type": "object",
                    "properties": {"expression": {"type": "string"}},
                    "required": ["expression"]
                }
            }
        }),
    ]
}

pub async fn execute(call: &ChatToolCall) -> String {
    let args: Value = match serde_json::from_str(&call.function.arguments) {
        Ok(value) => value,
        Err(error) => {
            return json!({"error": format!("invalid tool arguments: {error}")}).to_string();
        }
    };
    match call.function.name.as_str() {
        "get_weather" => {
            world::weather(
                text(&args, "location"),
                args.get("unit").and_then(Value::as_str),
            )
            .await
        }
        "get_time" => world::time(text(&args, "timezone")).await,
        "convert_units" => units::convert(
            args.get("value").and_then(Value::as_f64),
            text(&args, "from_unit"),
            text(&args, "to_unit"),
        ),
        "calculate" => calculate::evaluate(text(&args, "expression")),
        other => json!({"error": format!("unknown tool: {other}")}).to_string(),
    }
}

fn text<'a>(args: &'a Value, field: &str) -> &'a str {
    args.get(field).and_then(Value::as_str).unwrap_or("")
}
