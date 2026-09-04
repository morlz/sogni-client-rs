use anyhow::Result;
use futures_util::StreamExt;
use serde_json::Value;
use sogni_client::SogniClient;

pub(super) async fn stream(
    client: &SogniClient,
    id: &str,
    after: Option<&str>,
    last_event_id: Option<&str>,
) -> Result<()> {
    println!("Streaming events for {id}; resume cursor {last_event_id:?}");
    // Both values identify a replay position. `after` becomes the query cursor;
    // `last_event_id` also sets the standard SSE Last-Event-ID header.
    let mut stream = client
        .workflows
        .stream_events(id, after, last_event_id)
        .await?;
    while let Some(frame) = stream.next().await {
        let frame = frame?;
        println!(
            "[{}] {} {}",
            frame.id.as_deref().unwrap_or("-"),
            frame.event,
            frame.data
        );
    }
    Ok(())
}

pub(super) fn workflow_id(value: &Value) -> Option<&str> {
    value
        .get("workflowId")
        .or_else(|| value.get("workflow_id"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
}

pub(super) fn print_workflow(value: &Value) {
    println!(
        "Workflow: {}\nStatus:   {}",
        workflow_id(value).unwrap_or("unknown"),
        value.get("status").and_then(Value::as_str).unwrap_or("-")
    );
    if let Some(artifacts) = value.get("artifacts").and_then(Value::as_array) {
        for artifact in artifacts {
            println!("  artifact: {artifact}");
        }
    }
}
