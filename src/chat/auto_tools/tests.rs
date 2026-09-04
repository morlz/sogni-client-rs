use std::{
    collections::VecDeque,
    future::ready,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde_json::{Value, json};

use super::*;
use crate::ChatToolFunction;

fn tool_call(id: &str, name: &str) -> ChatToolCall {
    ChatToolCall {
        id: id.into(),
        call_type: "function".into(),
        function: ChatToolFunction {
            name: name.into(),
            arguments: "{}".into(),
        },
        extra: Default::default(),
    }
}

fn completion(
    job_id: &str,
    content: &str,
    finish_reason: &str,
    tool_calls: Vec<ChatToolCall>,
) -> ChatCompletion {
    ChatCompletion {
        job_id: job_id.into(),
        content: content.into(),
        role: "assistant".into(),
        finish_reason: finish_reason.into(),
        usage: json!({"prompt_tokens": 1, "completion_tokens": 1}),
        time_taken: 0.0,
        worker_name: None,
        cost: None,
        tool_calls,
        tool_history: None,
    }
}

fn params() -> Value {
    json!({
        "model": "fixture",
        "messages": [{"role": "user", "content": "run tools"}],
    })
}

#[test]
fn typed_loop_rejects_streaming_mode() {
    assert!(require_non_streaming(&json!({"stream": false})).is_ok());
    let error = require_non_streaming(&json!({"stream": true}))
        .expect_err("automatic execution is non-streaming");
    assert!(error.to_string().contains("stream=true"));
}

#[test]
fn child_attribution_links_each_followup_to_the_logical_round() {
    let logical = WorkloadAttribution {
        operation_id: Some("logical-operation".into()),
        ..WorkloadAttribution::default()
    };
    let child = auto_tool_child_attribution(&logical).expect("child attribution");
    assert_eq!(child.operation_scope, Some(OperationScope::Child));
    assert_eq!(child.operation_id, None);
    assert_eq!(
        child.root_operation_id.as_deref(),
        Some("logical-operation")
    );
    assert_eq!(
        child.parent_operation_id.as_deref(),
        Some("logical-operation")
    );
}

#[tokio::test]
async fn preserves_call_result_and_message_order() {
    let calls = vec![tool_call("call-b", "beta"), tool_call("call-a", "alpha")];
    let responses = Arc::new(Mutex::new(VecDeque::from([
        completion("one", "", "tool_calls", calls),
        completion("two", "done", "stop", Vec::new()),
    ])));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let executed = Arc::new(Mutex::new(Vec::new()));
    let executed_for_handler = executed.clone();
    let options = ChatAutoToolOptions::new(move |tool_call| {
        let executed = executed_for_handler.clone();
        async move {
            executed
                .lock()
                .expect("execution log")
                .push(tool_call.function.name.clone());
            Ok(format!("result-{}", tool_call.function.name))
        }
    });
    let requests_for_completion = requests.clone();
    let responses_for_completion = responses.clone();

    let result = drive_auto_tool_loop(params(), options, move |params, _, _| {
        requests_for_completion
            .lock()
            .expect("request log")
            .push(params);
        ready(Ok(responses_for_completion
            .lock()
            .expect("response queue")
            .pop_front()
            .expect("fixture completion")))
    })
    .await
    .expect("tool loop");

    assert_eq!(*executed.lock().expect("execution log"), ["beta", "alpha"]);
    let history = result.tool_history.expect("tool history");
    assert_eq!(history[0].tool_calls[0].id, "call-b");
    assert_eq!(history[0].tool_results[0].tool_call_id, "call-b");
    assert_eq!(history[0].tool_results[1].content, "result-alpha");
    let requests = requests.lock().expect("request log");
    let messages = requests[1]["messages"].as_array().expect("messages");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[2]["tool_call_id"], "call-b");
    assert_eq!(messages[3]["tool_call_id"], "call-a");
}

#[tokio::test]
async fn handler_errors_become_ordered_tool_results() {
    let responses = Arc::new(Mutex::new(VecDeque::from([
        completion(
            "one",
            "",
            "tool_calls",
            vec![tool_call("failed", "fallible")],
        ),
        completion("two", "recovered", "stop", Vec::new()),
    ])));
    let options =
        ChatAutoToolOptions::new(|_| async { Err(Error::InvalidInput("fixture failure".into())) });
    let result = drive_auto_tool_loop(params(), options, move |_, _, _| {
        ready(Ok(responses
            .lock()
            .expect("response queue")
            .pop_front()
            .expect("fixture completion")))
    })
    .await
    .expect("tool loop continues after a reported tool failure");

    let result = &result.tool_history.expect("history")[0].tool_results[0];
    assert!(!result.success);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("fixture failure"))
    );
    assert!(result.content.contains("fixture failure"));
}

#[tokio::test]
async fn enforces_round_limit() {
    let completions = Arc::new(AtomicUsize::new(0));
    let executions = Arc::new(AtomicUsize::new(0));
    let executions_for_handler = executions.clone();
    let options = ChatAutoToolOptions::new(move |_| {
        executions_for_handler.fetch_add(1, Ordering::SeqCst);
        ready(Ok("ok".into()))
    })
    .max_tool_rounds(2);
    let completions_for_loop = completions.clone();
    let error = drive_auto_tool_loop(params(), options, move |_, round, _| {
        completions_for_loop.fetch_add(1, Ordering::SeqCst);
        ready(Ok(completion(
            &round.to_string(),
            "",
            "tool_calls",
            vec![tool_call(&format!("call-{round}"), "repeat")],
        )))
    })
    .await
    .expect_err("loop must be bounded");

    assert!(error.to_string().contains("rounds (2) exceeded"));
    assert_eq!(completions.load(Ordering::SeqCst), 2);
    assert_eq!(executions.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn cancellation_stops_before_dispatch() {
    let cancellation = ChatAutoToolCancellation::new();
    cancellation.cancel();
    let completions = Arc::new(AtomicUsize::new(0));
    let completions_for_loop = completions.clone();
    let options =
        ChatAutoToolOptions::new(|_| ready(Ok("unused".into()))).cancellation(cancellation);
    let error = drive_auto_tool_loop(params(), options, move |_, _, _| {
        completions_for_loop.fetch_add(1, Ordering::SeqCst);
        ready(Ok(completion("unused", "", "stop", Vec::new())))
    })
    .await
    .expect_err("cancelled loop");

    assert!(error.to_string().contains("cancelled"));
    assert_eq!(completions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refuses_hosted_tools_without_invoking_custom_handler() {
    let executions = Arc::new(AtomicUsize::new(0));
    let executions_for_handler = executions.clone();
    let options = ChatAutoToolOptions::new(move |_| {
        executions_for_handler.fetch_add(1, Ordering::SeqCst);
        ready(Ok("must not execute".into()))
    });
    let error = drive_auto_tool_loop(params(), options, move |_, _, _| {
        ready(Ok(completion(
            "one",
            "",
            "tool_calls",
            vec![
                tool_call("custom", "local_lookup"),
                tool_call("hosted", "generate_image"),
            ],
        )))
    })
    .await
    .expect_err("hosted tools require server-side execution");

    assert!(error.to_string().contains("will not run it client-side"));
    assert_eq!(executions.load(Ordering::SeqCst), 0);
}
