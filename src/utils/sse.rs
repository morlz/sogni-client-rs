use serde_json::Value;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ParsedSseEvent {
    pub event: String,
    pub data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub raw: String,
}

#[must_use]
pub fn parse_sse_chunk(chunk: &str) -> Vec<ParsedSseEvent> {
    let normalized = chunk.replace("\r\n", "\n");
    normalized
        .split("\n\n")
        .filter_map(|raw| {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let mut event = "message".to_owned();
            let mut id = None;
            let mut data_lines = Vec::new();
            for line in raw.lines() {
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                let (field, value) = line.split_once(':').map_or((line, ""), |(field, value)| {
                    (field, value.strip_prefix(' ').unwrap_or(value))
                });
                match field {
                    "event" => event = if value.is_empty() { "message" } else { value }.to_owned(),
                    "id" => id = Some(value.to_owned()),
                    "data" => data_lines.push(value),
                    _ => {}
                }
            }
            let data = if data_lines.is_empty() {
                Value::Null
            } else {
                let joined = data_lines.join("\n");
                serde_json::from_str(&joined).unwrap_or(Value::String(joined))
            };
            Some(ParsedSseEvent {
                event,
                data,
                id,
                raw: raw.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sse_comments_crlf_multiline_json_and_text() {
        let frames = parse_sse_chunk(
            ": keep-alive\r\nid: 41\r\nevent: workflow_event\r\n\
             data: {\"status\":\"running\",\r\ndata: \"step\":\"image\"}\r\n\r\n\
             event:\ndata: not-json\n\n",
        );
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].id.as_deref(), Some("41"));
        assert_eq!(frames[0].event, "workflow_event");
        assert_eq!(
            frames[0].data,
            serde_json::json!({"status": "running", "step": "image"})
        );
        assert_eq!(frames[1].event, "message");
        assert_eq!(frames[1].data, "not-json");

        let empty = parse_sse_chunk("retry: 1000\ndata:\n\n");
        assert_eq!(empty[0].data, "");
    }
}
