use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

pub async fn weather(location: &str, unit: Option<&str>) -> String {
    if location.trim().is_empty() {
        return json!({"error": "location is required"}).to_string();
    }
    let mut url = match Url::parse("https://wttr.in/") {
        Ok(url) => url,
        Err(error) => return json!({"error": error.to_string()}).to_string(),
    };
    if let Ok(mut segments) = url.path_segments_mut() {
        segments.push(location.trim());
    }
    url.set_query(Some("format=j1"));
    let result = reqwest::Client::new()
        .get(url)
        .header("User-Agent", "sogni-client-rs-tool-example")
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    let response = match result {
        Ok(value) => value,
        Err(error) => {
            return json!({"error": format!("weather request failed: {error}")}).to_string();
        }
    };
    if !response.status().is_success() {
        return json!({"error": format!("weather service returned {}", response.status())})
            .to_string();
    }
    let payload: Value = match response.json().await {
        Ok(value) => value,
        Err(error) => {
            return json!({"error": format!("invalid weather response: {error}")}).to_string();
        }
    };
    let current = payload
        .pointer("/current_condition/0")
        .unwrap_or(&Value::Null);
    let country = payload
        .pointer("/nearest_area/0/country/0/value")
        .and_then(Value::as_str)
        .unwrap_or("");
    let fahrenheit =
        unit == Some("fahrenheit") || unit.is_none() && country == "United States of America";
    json!({
        "location": location,
        "country": country,
        "temperature": current.get(if fahrenheit { "temp_F" } else { "temp_C" }),
        "unit": if fahrenheit { "fahrenheit" } else { "celsius" },
        "feels_like": current.get(if fahrenheit { "FeelsLikeF" } else { "FeelsLikeC" }),
        "conditions": current.pointer("/weatherDesc/0/value"),
        "humidity_percent": current.get("humidity"),
        "wind_direction": current.get("winddir16Point"),
        "wind_speed": current.get(if fahrenheit { "windspeedMiles" } else { "windspeedKmph" }),
    })
    .to_string()
}

pub async fn time(input: &str) -> String {
    let timezone = city_timezone(input).unwrap_or(input.trim());
    if timezone.is_empty() {
        return json!({"error": "timezone is required"}).to_string();
    }
    let mut url = match Url::parse("https://worldtimeapi.org/api/timezone/") {
        Ok(url) => url,
        Err(error) => return json!({"error": error.to_string()}).to_string(),
    };
    if let Ok(mut segments) = url.path_segments_mut() {
        for segment in timezone.split('/') {
            segments.push(segment);
        }
    }
    let result = reqwest::Client::new()
        .get(url)
        .header("User-Agent", "sogni-client-rs-tool-example")
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => match response.json::<Value>().await {
            Ok(payload) => json!({
                "timezone": timezone,
                "datetime": payload.get("datetime"),
                "utc_offset": payload.get("utc_offset"),
                "abbreviation": payload.get("abbreviation"),
                "day_of_week": payload.get("day_of_week"),
            })
            .to_string(),
            Err(error) => json!({"error": format!("invalid time response: {error}")}).to_string(),
        },
        Ok(response) => {
            json!({"error": format!("unknown timezone {timezone:?}: HTTP {}", response.status())})
                .to_string()
        }
        Err(error) => json!({"error": format!("time request failed: {error}")}).to_string(),
    }
}

fn city_timezone(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "new york" | "nyc" => Some("America/New_York"),
        "los angeles" | "la" => Some("America/Los_Angeles"),
        "chicago" | "austin" | "dallas" => Some("America/Chicago"),
        "london" => Some("Europe/London"),
        "paris" => Some("Europe/Paris"),
        "berlin" => Some("Europe/Berlin"),
        "moscow" => Some("Europe/Moscow"),
        "dubai" => Some("Asia/Dubai"),
        "mumbai" | "delhi" => Some("Asia/Kolkata"),
        "singapore" => Some("Asia/Singapore"),
        "tokyo" | "osaka" => Some("Asia/Tokyo"),
        "seoul" => Some("Asia/Seoul"),
        "sydney" => Some("Australia/Sydney"),
        "auckland" => Some("Pacific/Auckland"),
        "toronto" => Some("America/Toronto"),
        "vancouver" => Some("America/Vancouver"),
        "sao paulo" | "são paulo" => Some("America/Sao_Paulo"),
        _ => None,
    }
}
