use serde_json::{Value, json};

use super::*;

fn completion_fixture() -> Value {
    json!({
        "jobID": "job-1",
        "content": "done",
        "role": "assistant",
        "finishReason": "stop",
        "usage": {"prompt_tokens": 1, "completion_tokens": 2},
        "timeTaken": 0.25,
    })
}

fn tool_call() -> Value {
    json!({
        "id": "call-1",
        "type": "function",
        "function": {"name": "lookup", "arguments": "{}"},
    })
}

#[test]
fn chunk_serializes_established_public_keys() {
    let chunk = ChatChunk {
        job_id: "job-1".into(),
        content: "partial".into(),
        role: Some("assistant".into()),
        finish_reason: Some("tool_calls".into()),
        usage: Some(json!({"completion_tokens": 1})),
        tool_calls: vec![tool_call()],
    };

    let value = serde_json::to_value(chunk).expect("serialize chat chunk");
    assert_eq!(value["jobID"], "job-1");
    assert_eq!(value["finishReason"], "tool_calls");
    assert_eq!(value["tool_calls"][0]["id"], "call-1");
    assert!(value.get("jobId").is_none());
    assert!(value.get("toolCalls").is_none());
}

#[test]
fn completion_serializes_established_public_keys() {
    let completion = ChatCompletion {
        job_id: "job-1".into(),
        content: "done".into(),
        role: "assistant".into(),
        finish_reason: "tool_calls".into(),
        usage: json!({"prompt_tokens": 1, "completion_tokens": 2}),
        time_taken: 0.25,
        worker_name: Some("worker".into()),
        cost: None,
        tool_calls: vec![serde_json::from_value(tool_call()).expect("tool call fixture")],
        tool_history: None,
    };

    let value = serde_json::to_value(completion).expect("serialize chat completion");
    assert_eq!(value["jobID"], "job-1");
    assert_eq!(value["finishReason"], "tool_calls");
    assert_eq!(value["timeTaken"], 0.25);
    assert_eq!(value["workerName"], "worker");
    assert_eq!(value["tool_calls"][0]["function"]["name"], "lookup");
    assert!(value.get("jobId").is_none());
    assert!(value.get("toolCalls").is_none());
}

#[test]
fn chunk_accepts_absent_null_and_legacy_tool_calls() {
    let absent: ChatChunk = serde_json::from_value(json!({
        "jobID": "job-1",
        "content": "",
    }))
    .expect("chunk without tool calls");
    assert!(absent.tool_calls.is_empty());

    let null: ChatChunk = serde_json::from_value(json!({
        "jobID": "job-1",
        "content": "",
        "tool_calls": null,
    }))
    .expect("chunk with null tool calls");
    assert!(null.tool_calls.is_empty());

    let legacy: ChatChunk = serde_json::from_value(json!({
        "jobId": "legacy-job",
        "content": "",
        "toolCalls": [tool_call()],
    }))
    .expect("legacy Rust chunk shape");
    assert_eq!(legacy.job_id, "legacy-job");
    assert_eq!(legacy.tool_calls[0]["id"], "call-1");
}

#[test]
fn completion_accepts_absent_null_and_legacy_tool_calls() {
    let absent: ChatCompletion =
        serde_json::from_value(completion_fixture()).expect("completion without tool calls");
    assert!(absent.tool_calls.is_empty());

    let mut null_fixture = completion_fixture();
    null_fixture["tool_calls"] = Value::Null;
    let null: ChatCompletion =
        serde_json::from_value(null_fixture).expect("completion with null tool calls");
    assert!(null.tool_calls.is_empty());

    let mut legacy_fixture = completion_fixture();
    let object = legacy_fixture.as_object_mut().expect("completion object");
    let job_id = object.remove("jobID").expect("job ID");
    object.insert("jobId".into(), job_id);
    object.insert("toolCalls".into(), json!([tool_call()]));
    let legacy: ChatCompletion =
        serde_json::from_value(legacy_fixture).expect("legacy Rust completion shape");
    assert_eq!(legacy.job_id, "job-1");
    assert_eq!(legacy.tool_calls[0].function.name, "lookup");
}
