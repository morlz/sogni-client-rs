use std::{
    convert::Infallible,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    http::{HeaderMap, Method, StatusCode, Uri, header::CONTENT_TYPE},
    response::IntoResponse,
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use serde_json::{Value, json};
use sogni_client::{
    Attribution, ConfirmWorldBuildCostParams, Error, ListWorldBuildsOptions,
    ReviewWorldBuildTaskParams, SogniClient, StartWorldBuildParams, StreamWorldBuildEventsOptions,
    WorkloadAttribution, WorkloadKind, WorldBuildCostDecision, WorldBuildEventCancellation,
    WorldBuildRecord, WorldBuildReviewDecision, WorldBuildStatus, WorldBuildWaitingReason,
};
use tokio::{
    net::TcpListener,
    sync::{Notify, mpsc},
    task::JoinHandle,
};

const WATCHDOG: Duration = Duration::from_secs(10);

fn run() -> Value {
    json!({
        "runId": "wbuild / 1", "ownerWalletAddress": "fixture-owner", "worldId": "w", "nodeId": "n",
        "status": "waiting_for_user",
        "scope": {"ownerWalletAddress": "fixture-owner", "appSource": "sogni-world", "tokenType": "spark", "billingMode": "subscription"},
        "input": {"worldId": "w", "nodeId": "n", "look": "painted", "audience": "teen", "hotspots": [
            {"taskId": "door", "kind": "path", "object": "the door", "action": {"verb": "Open", "subject": "the door", "intent": "Step through", "tone": "gentle"}, "leadsTo": "Garden", "duration": 5.17, "point": {"x": 0.4, "y": 0.6}}
        ], "plan": {"publicPlanMetadata": true}},
        "tasks": [{
            "taskId": "door", "kind": "path", "status": "review", "attempts": 1,
            "selection": {"id": "selection", "selectionHash": "hash", "projectId": "select", "points": [{"x": 0.4, "y": 0.6, "label": "positive"}], "bounds": {"x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4}, "coverage": 0.1},
            "generationId": "gen", "interactionId": "interaction",
            "projects": {"segmentation": "select", "target_still": "still", "transition": "video", "interaction": "moment"},
            "direction": "Open the door", "review": {"decision": "rejected", "note": "too dark", "at": "t"},
            "error": {"message": "Try again", "code": "unavailable", "at": "t", "step": "rendering_transition"},
            "startedAt": "t", "finishedAt": "t", "futurePublicMetadata": true
        }],
        "costPreview": {"tokenType": "spark", "billingMode": "subscription", "lines": [
            {"taskId": "door", "kind": "path", "stages": {"transition": 1.25}, "total": 1.25}
        ], "total": 1.25, "coveredBySubscription": true, "issuedAt": "t", "validUntil": "later"},
        "authorization": {"acceptedTotal": 1.25, "tokenType": "spark", "previewIssuedAt": "t", "acceptedAt": "t"},
        "waiting": {"reason": "review_required", "message": "Review this take", "details": {"taskId": "door"}, "since": "t"},
        "events": [{"sequence": 1, "type": "task_ready", "at": "t", "payload": {"taskId": "door"}}],
        "failureReason": "unavailable", "cancellationReason": "caller cancelled",
        "timestamps": {"createdAt": "t", "updatedAt": "t", "completedAt": "t"},
        "futurePublicField": {"kept": true}
    })
}

async fn server(router: Router) -> (url::Url, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, task)
}

async fn client(url: url::Url) -> SogniClient {
    SogniClient::builder()
        .disable_socket(true)
        .api_key("local-fixture")
        .rest_endpoint(url)
        .build()
        .await
        .unwrap()
}

#[derive(Debug)]
struct Call {
    method: Method,
    uri: String,
    headers: HeaderMap,
    body: Value,
}

#[tokio::test]
async fn world_build_requests_preserve_wire_shapes_decisions_and_header_only_idempotency() {
    let (tx, mut calls) = mpsc::unbounded_channel();
    let router = account_router().fallback(
        move |method: Method, uri: Uri, headers: HeaderMap, bytes: Bytes| {
            let tx = tx.clone();
            async move {
                let body = if bytes.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                tx.send(Call {
                    method,
                    uri: uri.to_string(),
                    headers,
                    body,
                })
                .unwrap();
                let data = if uri.path().ends_with("/events") {
                    json!({"events": [{"sequence": 3, "type": "task_step", "at": "now"}]})
                } else if uri.path() == "/v1/world-builds" && uri.query().is_some() {
                    json!({"runs": [run()]})
                } else {
                    json!({"run": run(), "idempotent": false})
                };
                (
                    StatusCode::ACCEPTED,
                    Json(json!({"status": "success", "data": data})),
                )
            }
        },
    );
    let (url, server) = server(router).await;
    let client = SogniClient::builder()
        .disable_socket(true)
        .api_key("local-fixture")
        .app_source("sogni-world")
        .attribution(Attribution {
            workload: Some(WorkloadAttribution {
                workload_kind: Some(WorkloadKind::Direct),
                ..Default::default()
            }),
            ..Default::default()
        })
        .rest_endpoint(url)
        .build()
        .await
        .unwrap();
    let input = json!({
        "worldId": "w", "nodeId": "n", "look": "painted", "audience": "teen", "tokenType": "spark", "billingMode": "subscription",
        "hotspots": [{"kind": "path", "object": "the door", "action": {"verb": "Open", "subject": "the door", "intent": "Step through"}, "leadsTo": "Garden", "duration": 5.17, "point": {"x": 0.4, "y": 0.6}}]
    });
    let mut params: StartWorldBuildParams = serde_json::from_value(input.clone()).unwrap();
    params.idempotency_key = Some("once".into());
    let record = client.worlds.builds.start(params).await.unwrap();
    assert_eq!(record.status, WorldBuildStatus::WaitingForUser);
    assert_eq!(
        record.waiting.unwrap().reason,
        WorldBuildWaitingReason::ReviewRequired
    );
    let start = calls.try_recv().unwrap();
    assert_eq!(start.method, Method::POST);
    assert_eq!(start.uri, "/v1/world-builds");
    assert_eq!(start.body, input);
    assert_eq!(start.headers["idempotency-key"], "once");
    assert_eq!(start.headers["api-key"], "local-fixture");
    assert_eq!(start.headers["x-app-source"], "sogni-world");
    assert_eq!(start.headers["x-sogni-workload-kind"], "direct");
    assert!(!start.headers["x-sogni-operation-id"].is_empty());
    assert!(
        calls.try_recv().is_err(),
        "starting must not implicitly confirm or review"
    );

    assert_eq!(
        client
            .worlds
            .builds
            .list(ListWorldBuildsOptions {
                world_id: Some("w / 1".into()),
                limit: Some(0)
            })
            .await
            .unwrap()
            .len(),
        1
    );
    let list = calls.try_recv().unwrap();
    assert_eq!(list.method, Method::GET);
    let query: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(list.uri.split_once('?').unwrap().1.as_bytes())
            .into_owned()
            .collect();
    assert_eq!(query["worldId"], "w / 1");
    assert_eq!(query["limit"], "0");
    client.worlds.builds.get("wbuild / 1").await.unwrap();
    assert_eq!(
        calls.try_recv().unwrap().uri,
        "/v1/world-builds/wbuild%20%2F%201"
    );
    assert_eq!(
        client
            .worlds
            .builds
            .events("wbuild / 1", Some(0))
            .await
            .unwrap()[0]
            .sequence,
        3
    );
    assert_eq!(
        calls.try_recv().unwrap().uri,
        "/v1/world-builds/wbuild%20%2F%201/events?after=0"
    );
    client
        .worlds
        .builds
        .events("wbuild / 1", None)
        .await
        .unwrap();
    assert_eq!(
        calls.try_recv().unwrap().uri,
        "/v1/world-builds/wbuild%20%2F%201/events"
    );

    for decision in [
        WorldBuildCostDecision::Confirm,
        WorldBuildCostDecision::Cancel,
        WorldBuildCostDecision::Requote,
    ] {
        client
            .worlds
            .builds
            .confirm_cost("wbuild / 1", ConfirmWorldBuildCostParams { decision })
            .await
            .unwrap();
        let call = calls.try_recv().unwrap();
        assert_eq!(call.method, Method::POST);
        assert_eq!(call.uri, "/v1/world-builds/wbuild%20%2F%201/confirm-cost");
        assert_eq!(call.body, json!({"decision": decision}));
        assert!(!call.headers.contains_key("idempotency-key"));
    }
    for (decision, note) in [
        (WorldBuildReviewDecision::Rejected, Some("too dark")),
        (WorldBuildReviewDecision::Approved, None),
    ] {
        client
            .worlds
            .builds
            .review(
                "wbuild / 1",
                ReviewWorldBuildTaskParams {
                    task_id: "door".into(),
                    decision,
                    note: note.map(str::to_owned),
                },
            )
            .await
            .unwrap();
        let call = calls.try_recv().unwrap();
        assert_eq!(call.uri, "/v1/world-builds/wbuild%20%2F%201/review");
        let expected = if let Some(note) = note {
            json!({"taskId": "door", "decision": decision, "note": note})
        } else {
            json!({"taskId": "door", "decision": decision})
        };
        assert_eq!(call.body, expected);
    }
    for reason in [Some("changed my mind"), None, Some("")] {
        client
            .worlds
            .builds
            .cancel("wbuild / 1", reason)
            .await
            .unwrap();
        let call = calls.try_recv().unwrap();
        assert_eq!(call.uri, "/v1/world-builds/wbuild%20%2F%201/cancel");
        assert_eq!(
            call.body,
            reason
                .filter(|r| !r.is_empty())
                .map_or_else(|| json!({}), |r| json!({"reason": r}))
        );
    }
    client.abort();
    server.abort();
}

#[test]
fn world_build_record_roundtrips_all_public_fields_and_omitted_list_events() {
    let complete = run();
    let record: WorldBuildRecord = serde_json::from_value(complete.clone()).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), complete);
    let mut listed = complete;
    listed.as_object_mut().unwrap().remove("events");
    let record: WorldBuildRecord = serde_json::from_value(listed.clone()).unwrap();
    assert!(record.events.is_none());
    assert_eq!(serde_json::to_value(record).unwrap(), listed);
}

#[tokio::test]
async fn world_build_errors_keep_server_authority_message_retry_and_details() {
    let router = account_router().fallback(|method: Method, uri: Uri, headers: HeaderMap| async move {
        assert_eq!(headers["api-key"], "local-fixture");
        assert_eq!(method, Method::POST);
        if uri.path() == "/v1/world-builds" {
            (StatusCode::FORBIDDEN, Json(json!({"status": "error", "message": "Sign in to start a world build.", "details": {"signInRequired": true}}))).into_response()
        } else if uri.path().ends_with("/confirm-cost") {
            (StatusCode::CONFLICT, Json(json!({"status": "error", "message": "That quote has expired. Ask for a fresh one with decision requote."}))).into_response()
        } else {
            (StatusCode::TOO_MANY_REQUESTS, [("Retry-After", "17")], Json(json!({"status": "error", "message": "Wait", "retryAfter": 2.5, "details": {"tryLater": true}}))).into_response()
        }
    });
    let (url, server) = server(router).await;
    let client = client(url).await;
    let error = client
        .worlds
        .builds
        .start(StartWorldBuildParams {
            world_id: "w".into(),
            node_id: "n".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    let Error::Api(error) = error else {
        panic!("expected server refusal")
    };
    assert_eq!(error.status, 403);
    assert!(error.message.contains("Sign in"));
    assert_eq!(error.details().unwrap()["signInRequired"], true);
    let error = client
        .worlds
        .builds
        .confirm_cost(
            "run",
            ConfirmWorldBuildCostParams {
                decision: WorldBuildCostDecision::Confirm,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Api(error) if error.status == 409 && error.message.contains("quote has expired"))
    );
    let error = client.worlds.builds.cancel("run", None).await.unwrap_err();
    let Error::Api(error) = error else {
        panic!("expected rate limit")
    };
    assert_eq!(error.status, 429);
    assert_eq!(error.retry_after(), Some(2.5));
    assert_eq!(error.retry_after_seconds, Some(3));
    assert_eq!(error.details().unwrap()["tryLater"], true);
    client.abort();
    server.abort();
}

#[tokio::test]
async fn world_build_sse_replays_zero_and_parses_split_crlf_multiline_and_final_frames() {
    let (tx, mut requests) = mpsc::unbounded_channel();
    let router = account_router().fallback(move |uri: Uri, headers: HeaderMap| {
        let tx = tx.clone();
        async move {
            tx.send((uri.to_string(), headers)).unwrap();
            let source = concat!(
                ": keep-alive\r\n\r\n",
                "id: 1\r\nevent: run_started\r\ndata: {\"sequence\":1,\r\ndata: \"type\":\"run_started\",\"at\":\"t\",\"payload\":{\"title\":\"Дверь\"}}\r\n\r\n",
                "event: run_status\ndata: {\"sequence\":99,\"type\":\"snapshot\",\"at\":\"t\"}\n\n",
                "event: task_ready\ndata: not-json\n\n",
                "data: {}\n\n",
                "data: []\n\n",
                "id: 2\nevent: task_ready\ndata: {\"sequence\":2,\"type\":\"task_ready\",\"at\":\"t\",\"payload\":{\"taskId\":\"door\"}}\n\n",
                "id: 3\nevent: run_completed\ndata: {\"sequence\":3,\"type\":\"run_completed\",\"at\":\"t\"}"
            );
            let chunks = source.as_bytes().chunks(3).map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk))).collect::<Vec<_>>();
            ([(CONTENT_TYPE, "text/event-stream")], Body::from_stream(futures_util::stream::iter(chunks)))
        }
    });
    let (url, server) = server(router).await;
    let client = client(url).await;
    let mut events = client
        .worlds
        .builds
        .stream_events(
            "wbuild / 1",
            StreamWorldBuildEventsOptions {
                last_event_id: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut received = Vec::new();
    while let Some(event) = events.next().await {
        received.push(event.unwrap());
    }
    assert_eq!(
        received
            .iter()
            .map(|e| (e.sequence, e.event_type.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "run_started"), (2, "task_ready"), (3, "run_completed")]
    );
    assert_eq!(received[0].payload.as_ref().unwrap()["title"], "Дверь");
    let (uri, headers) = requests.try_recv().unwrap();
    assert_eq!(uri, "/v1/world-builds/wbuild%20%2F%201/events/stream");
    assert_eq!(headers["last-event-id"], "0");
    assert_eq!(headers["accept"], "text/event-stream");
    assert_eq!(headers["api-key"], "local-fixture");
    client.abort();
    server.abort();
}

#[tokio::test]
async fn world_build_sse_errors_preserve_retry_header_without_guessing_events() {
    let router = account_router().fallback(|| async {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [("Retry-After", "17")],
            Json(json!({"status": "error", "message": "Wait"})),
        )
    });
    let (url, server) = server(router).await;
    let client = client(url).await;
    let result = client
        .worlds
        .builds
        .stream_events("run", Default::default())
        .await;
    let Err(Error::Api(error)) = result else {
        panic!("expected stream HTTP error")
    };
    assert_eq!(error.status, 429);
    assert_eq!(error.retry_after_seconds, Some(17));
    client.abort();
    server.abort();
}

fn account_router() -> Router {
    Router::new().route("/v1/account/me", get(|| async { Json(json!({"data": {"username": "fixture"}})) }))
        .fallback(|| async { Json(json!({"status": "success", "data": {}, "activeProjects": [], "unclaimedCompletedProjects": []})) })
}

fn token(user: &str) -> String {
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        + 3600.0;
    format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(json!({"sub": user, "exp": expiry}).to_string())
    )
}

#[tokio::test]
async fn world_build_json_body_is_cancelled_on_account_change_and_client_close() {
    for close in [false, true] {
        let begun = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let signal = begun.clone();
        let wait = release.clone();
        let router = account_router().route("/v1/world-builds/held", get(move || {
            let signal = signal.clone(); let wait = wait.clone();
            async move {
                let body = async_stream::stream! {
                    yield Ok::<_, Infallible>(Bytes::from_static(b"{\"status\":\"success\",\"data\":"));
                    signal.notify_one();
                    wait.notified().await;
                    yield Ok(Bytes::from(format!("{}}}", json!({"run": run()}))));
                };
                ([(CONTENT_TYPE, "application/json")], Body::from_stream(body))
            }
        }));
        let (url, server) = server(router).await;
        let client = SogniClient::builder()
            .disable_socket(true)
            .rest_endpoint(url)
            .build()
            .await
            .unwrap();
        client
            .set_tokens(token("a"), token("refresh-a"))
            .await
            .unwrap();
        let builds = client.worlds.builds.clone();
        let request = tokio::spawn(async move { builds.get("held").await });
        tokio::time::timeout(WATCHDOG, begun.notified())
            .await
            .unwrap();
        if close {
            client.close().await.unwrap();
        } else {
            client
                .set_tokens(token("b"), token("refresh-b"))
                .await
                .unwrap();
        }
        let error = tokio::time::timeout(WATCHDOG, request)
            .await
            .expect("cancel the blocked body promptly")
            .unwrap()
            .unwrap_err();
        if close {
            assert!(matches!(error, Error::Closed));
        } else {
            assert!(
                matches!(error, Error::InvalidInput(message) if message.contains("account session changed"))
            );
        }
        release.notify_one();
        client.abort();
        server.abort();
    }
}

#[tokio::test]
async fn idle_world_build_events_stop_on_session_change_close_or_local_cancellation() {
    for mode in 0..3 {
        let router = account_router().route("/v1/world-builds/run/events/stream", get(|| async {
            let body = async_stream::stream! {
                yield Ok::<_, Infallible>(Bytes::from_static(b"data: {\"sequence\":1,\"type\":\"run_started\",\"at\":\"t\"}\n\n"));
                std::future::pending::<()>().await;
            };
            ([(CONTENT_TYPE, "text/event-stream")], Body::from_stream(body))
        }));
        let (url, server) = server(router).await;
        let client = SogniClient::builder()
            .disable_socket(true)
            .rest_endpoint(url)
            .build()
            .await
            .unwrap();
        client
            .set_tokens(token("a"), token("refresh-a"))
            .await
            .unwrap();
        let cancellation = WorldBuildEventCancellation::new();
        let mut events = client
            .worlds
            .builds
            .stream_events(
                "run",
                StreamWorldBuildEventsOptions {
                    cancellation: Some(cancellation.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(events.next().await.unwrap().unwrap().sequence, 1);
        let next = events.next();
        tokio::pin!(next);
        assert!(futures_util::poll!(next.as_mut()).is_pending());
        match mode {
            0 => client
                .set_tokens(token("b"), token("refresh-b"))
                .await
                .unwrap(),
            1 => client.close().await.unwrap(),
            _ => cancellation.cancel(),
        }
        let error = tokio::time::timeout(WATCHDOG, next)
            .await
            .expect("cancel the idle stream promptly")
            .unwrap()
            .unwrap_err();
        match mode {
            0 => assert!(
                matches!(error, Error::InvalidInput(message) if message.contains("account session changed"))
            ),
            1 => assert!(matches!(error, Error::Closed)),
            _ => {
                assert!(matches!(error, Error::Transport(message) if message.contains("cancelled")))
            }
        }
        client.abort();
        server.abort();
    }
}

#[tokio::test]
async fn world_build_event_cancellation_prevents_a_new_request() {
    let (tx, mut calls) = mpsc::unbounded_channel();
    let (url, server) = server(account_router().fallback(move || {
        let tx = tx.clone();
        async move {
            tx.send(()).unwrap();
            Json(json!({}))
        }
    }))
    .await;
    let client = client(url).await;
    let cancellation = WorldBuildEventCancellation::new();
    cancellation.cancel();
    let result = client
        .worlds
        .builds
        .stream_events(
            "run",
            StreamWorldBuildEventsOptions {
                cancellation: Some(cancellation),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(result, Err(Error::Transport(message)) if message.contains("cancelled")));
    assert!(calls.try_recv().is_err());
    client.abort();
    server.abort();
}

#[tokio::test]
async fn world_build_start_rejects_invalid_idempotency_header_before_sending() {
    let (tx, mut calls) = mpsc::unbounded_channel();
    let (url, server) = server(account_router().fallback(move || {
        let tx = tx.clone();
        async move {
            tx.send(()).unwrap();
            Json(json!({}))
        }
    }))
    .await;
    let client = client(url).await;
    let result = client
        .worlds
        .builds
        .start(StartWorldBuildParams {
            world_id: "w".into(),
            node_id: "n".into(),
            idempotency_key: Some("key\r\nInjected: header".into()),
            ..Default::default()
        })
        .await;
    assert!(matches!(result, Err(Error::InvalidInput(_))));
    assert!(calls.try_recv().is_err());
    client.abort();
    server.abort();
}
