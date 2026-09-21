//! Public HTTP contracts introduced in TypeScript SDK 5.51–5.54.
use std::{collections::BTreeMap, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sogni_client::{Error, ImportPersonalLoraParams, Network, SogniClient, WorkflowBillingOptions};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};

#[derive(Clone, Debug)]
struct Capture {
    method: String,
    path: String,
    query: BTreeMap<String, String>,
    headers: HeaderMap,
    body: Value,
}

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Value,
    retry_after: Option<String>,
}

#[derive(Default)]
struct FixtureState {
    requests: Mutex<Vec<Capture>>,
    replies: Mutex<BTreeMap<String, Reply>>,
    hold_path: Mutex<Option<String>>,
    started: Notify,
    release: Notify,
}

struct Fixture {
    state: Arc<FixtureState>,
    client: SogniClient,
    task: JoinHandle<()>,
}

impl Fixture {
    async fn start() -> Self {
        let state = Arc::new(FixtureState::default());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new().fallback(serve).with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = SogniClient::builder()
            .app_id("upstream-contract-fixture")
            .api_key("local-contract-fixture")
            .network(Network::Relaxed)
            .rest_endpoint(url.parse().unwrap())
            .socket_endpoint(url.replace("http:", "ws:").parse().unwrap())
            .defer_socket_start(true)
            .build()
            .await
            .unwrap();
        Self {
            state,
            client,
            task,
        }
    }

    fn reply(&self, path: &str, body: Value) {
        self.state.replies.lock().insert(
            path.into(),
            Reply {
                status: StatusCode::OK,
                body,
                retry_after: None,
            },
        );
    }

    fn error(&self, path: &str, body: Value, retry_after: &str) {
        self.state.replies.lock().insert(
            path.into(),
            Reply {
                status: StatusCode::TOO_MANY_REQUESTS,
                body,
                retry_after: Some(retry_after.into()),
            },
        );
    }

    fn requests(&self, path: &str) -> Vec<Capture> {
        self.state
            .requests
            .lock()
            .iter()
            .filter(|request| request.path == path)
            .cloned()
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(State(state): State<Arc<FixtureState>>, request: Request<Body>) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path().to_owned();
    let body = to_bytes(body, 1024 * 1024).await.unwrap();
    state.requests.lock().push(Capture {
        method: parts.method.to_string(),
        path: path.clone(),
        query: url::form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes())
            .into_owned()
            .collect(),
        headers: parts.headers,
        body: if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    });
    let hold = state.hold_path.lock().as_deref() == Some(&path);
    if hold {
        state.started.notify_one();
        state.release.notified().await;
    }
    let reply = state.replies.lock().get(&path).cloned();
    if let Some(reply) = reply {
        let mut response = (reply.status, Json(reply.body)).into_response();
        if let Some(retry_after) = reply.retry_after {
            response
                .headers_mut()
                .insert("retry-after", retry_after.parse().unwrap());
        }
        return response;
    }
    let value = match path.as_str() {
        "/v1/account/me" => {
            json!({"status":"success","data":{"username":"fixture","email":"fixture@example.test"}})
        }
        "/v1/account/logout" => json!({"status":"success","data":{}}),
        "/api/v1/models/list" => json!([{"id":"fixture-model","tier":"fixture"}]),
        "/api/v2/models/tiers" => json!({"fixture":{"type":"image"}}),
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"message":"fixture route missing"})),
            )
                .into_response();
        }
    };
    Json(value).into_response()
}

#[tokio::test]
async fn cost_approval_preserves_the_exact_preview_and_never_accepts_a_replacement() {
    let fixture = Fixture::start().await;
    let path = "/v1/chat/runs/run%2F1/confirm-cost";
    fixture.reply(path, json!({"data":{"run":{"id":"run/1"}}}));
    let preview = json!({"version":2,"cost":1.25,"tokenType":"spark","future":{"nullable":null},"items":[null,{"cost":0}]});
    fixture
        .client
        .chat
        .confirm_run_cost(
            "run/1",
            &json!({
                "toolCallId":"tool-1","decision":"confirm","acceptedCostPreview":preview,
                "idempotencyKey":"approve-once"
            }),
        )
        .await
        .unwrap();
    let request = &fixture.requests(path)[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.headers["idempotency-key"], "approve-once");
    assert_eq!(
        request.body,
        json!({"tool_call_id":"tool-1","decision":"confirm","acceptedCostPreview":preview})
    );
    fixture
        .client
        .chat
        .confirm_run_cost("run/1", &json!({"toolCallId":"tool-1","decision":"cancel"}))
        .await
        .unwrap();
    assert_eq!(
        fixture.requests(path)[1].body,
        json!({"tool_call_id":"tool-1","decision":"cancel"})
    );

    fixture.state.replies.lock().insert(
        path.into(),
        Reply {
            status: StatusCode::CONFLICT,
            body: json!({"error":"stale_preview","details":{"costPreview":{"cost":99}}}),
            retry_after: None,
        },
    );
    assert!(
        fixture
            .client
            .chat
            .confirm_run_cost(
                "run/1",
                &json!({
                    "toolCallId":"tool-1","decision":"confirm","acceptedCostPreview":preview
                })
            )
            .await
            .is_err()
    );
    assert_eq!(
        fixture.requests(path).len(),
        3,
        "no automatic acceptance retry"
    );
    assert!(
        fixture.requests("/v1/chat/runs/run%2F1").is_empty(),
        "no automatic preview fetch"
    );
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn reseed_sends_idempotency_as_a_header_and_preserves_replayed_metadata() {
    let fixture = Fixture::start().await;
    let path = "/v1/creative-agent/workflows/source/reseed";
    fixture.reply(path, json!({"data":{"workflow":{"id":"cloned"},"reseed":{"cloned_from_run_id":"source","steps":[{"id":"step","seed":42}]},"idempotent":true}}));
    let result = fixture
        .client
        .workflows
        .reseed(
            "source",
            WorkflowBillingOptions {
                billing_mode: Some("auto".into()),
                idempotency_key: Some("same-take".into()),
                ..Default::default()
            },
            Some(json!({"step":42})),
        )
        .await
        .unwrap();
    assert_eq!(result.idempotent, Some(true));
    assert_eq!(result.reseed.cloned_from_run_id, "source");
    assert_eq!(result.reseed.steps[0]["seed"], 42);
    let request = &fixture.requests(path)[0];
    assert_eq!(request.headers["idempotency-key"], "same-take");
    assert_eq!(
        request.body,
        json!({"billing_mode":"auto","seed_overrides":{"step":42}})
    );
    fixture.reply(path, json!({"data":{"workflow":{"id":"another"},"reseed":{"cloned_from_run_id":"source","steps":[]}}}));
    assert_eq!(
        fixture
            .client
            .workflows
            .reseed("source", Default::default(), None)
            .await
            .unwrap()
            .idempotent,
        None
    );
    assert!(
        !fixture.requests(path)[1]
            .headers
            .contains_key("idempotency-key")
    );
    fixture.client.close().await.unwrap();
}

fn assert_retry(error: Error, expected: f64) {
    match error {
        Error::Api(error) => {
            assert_eq!(error.retry_after(), Some(expected));
            assert_eq!(error.details().unwrap()["hint"], "wait");
        }
        Error::Chat(error) => {
            assert_eq!(error.retry_after(), Some(expected));
            assert_eq!(error.details().unwrap()["hint"], "wait");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn retry_guidance_survives_every_rest_facade() {
    let fixture = Fixture::start().await;
    for path in [
        "/v1/chat/runs/busy",
        "/v1/creative-agent/workflows/busy",
        "/v1/creative-agent/workflows/templates",
        "/v1/replay/records/busy",
    ] {
        fixture.error(
            path,
            json!({"message":"busy","retryAfter":2.5,"details":{"hint":"wait"}}),
            "99",
        );
    }
    assert_retry(fixture.client.chat.get_run("busy").await.unwrap_err(), 2.5);
    assert_retry(fixture.client.workflows.get("busy").await.unwrap_err(), 2.5);
    assert_retry(
        fixture
            .client
            .workflows
            .templates
            .list(None, None, None)
            .await
            .unwrap_err(),
        2.5,
    );
    assert_retry(fixture.client.replay.get("busy").await.unwrap_err(), 2.5);
    fixture.error(
        "/v1/creative-agent/workflows/busy",
        json!({"data":{"message":"busy","retryAfter":1.5,"details":{"hint":"wait"}}}),
        "99",
    );
    assert_retry(fixture.client.workflows.get("busy").await.unwrap_err(), 1.5);
    let path = "/v1/chat/runs/busy";
    fixture.error(
        path,
        json!({"message":"busy","retryAfter":"invalid","details":{"hint":"wait"}}),
        "42",
    );
    assert_retry(fixture.client.chat.get_run("busy").await.unwrap_err(), 42.0);
    fixture.error(
        path,
        json!({"message":"busy","retryAfter":0,"details":{"hint":"wait"}}),
        "42",
    );
    assert_retry(fixture.client.chat.get_run("busy").await.unwrap_err(), 0.0);
    fixture.error(
        path,
        json!({"message":"busy","details":{"hint":"wait"}}),
        "Sun, 06 Sep 2026 12:00:00 GMT",
    );
    assert_retry(fixture.client.chat.get_run("busy").await.unwrap_err(), 0.0);
    fixture.client.close().await.unwrap();
}

fn personal_entry() -> Value {
    json!({"id":"personal-a","name":"My adapter","modelId":"model-a","modelIds":["model-a"],"source":"huggingface","status":"queued","createdAt":1,"updatedAt":2,"requirements":[]})
}

#[tokio::test]
async fn personal_loras_use_private_routes_without_polluting_the_public_catalog() {
    let fixture = Fixture::start().await;
    let path = "/v1/loras/personal";
    fixture.reply(path, json!({"data":{"loras":[personal_entry()],"models":["model-a"],"limits":{"entries":5,"fileBytes":null,"importsPerDay":3,"perGeneration":2}}}));
    let library = fixture
        .client
        .projects
        .personal_loras()
        .list()
        .await
        .unwrap();
    assert_eq!(library.loras[0].status, "queued");
    assert_eq!(library.limits.file_bytes, None);
    fixture.reply(path, json!({"data":personal_entry()}));
    fixture
        .client
        .projects
        .personal_loras()
        .import(&ImportPersonalLoraParams {
            url: "https://huggingface.co/example/adapter/resolve/main/adapter.safetensors".into(),
            name: "My adapter".into(),
            model_id: "model-a".into(),
            rights_confirmed: false,
        })
        .await
        .unwrap();
    assert_eq!(
        fixture.requests(path)[1].body["rightsConfirmed"],
        false,
        "permission is never inferred"
    );
    let encoded = "/v1/loras/personal/personal-a%2Fother";
    fixture.reply(encoded, json!({"data":personal_entry()}));
    fixture
        .client
        .projects
        .personal_loras()
        .get("personal-a/other")
        .await
        .unwrap();
    fixture
        .client
        .projects
        .personal_loras()
        .remove("personal-a/other")
        .await
        .unwrap();
    assert_eq!(fixture.requests(encoded)[1].method, "DELETE");

    let public = json!({"loras":[{"loraId":"public","modelIds":["model-a"]}],"models":["model-a"]});
    fixture.reply("/v1/loras/comfy", json!({"data":public}));
    let private_path = "/v1/loras/personal/catalog";
    fixture.reply(
        private_path,
        json!({"data":{"loras":[
            {"loraId":"personal-a","modelIds":["model-a"],"strengthMin":0,"strengthMax":2},
            {"loraId":"personal-b","modelIds":["model-b"]}
        ]}}),
    );
    let merged = fixture
        .client
        .projects
        .available_loras_with_personal(Some("model-a"))
        .await
        .unwrap();
    assert_eq!(merged["loras"].as_array().unwrap().len(), 2);
    assert_eq!(merged["models"], json!(["model-a", "model-b"]));
    assert_eq!(
        fixture
            .client
            .projects
            .get_lora("personal-a")
            .await
            .unwrap()
            .unwrap()["strengthMax"],
        2
    );
    fixture.reply(private_path, json!({"data":{"loras":[]}}));
    assert!(
        fixture
            .client
            .projects
            .get_lora("personal-a")
            .await
            .unwrap()
            .is_none(),
        "private catalog is never cached"
    );
    assert_eq!(
        fixture.client.projects.available_loras(None).await.unwrap(),
        public
    );
    assert_eq!(fixture.requests(private_path).len(), 3);
    fixture.error(private_path, json!({"message":"unavailable"}), "1");
    assert!(
        fixture
            .client
            .projects
            .available_loras_with_personal(None)
            .await
            .is_err(),
        "private errors must not silently look like an empty library"
    );
    fixture.reply("/v1/loras/comfy", json!({"data":[]}));
    assert!(
        matches!(
            fixture
                .client
                .projects
                .available_loras_with_personal(None)
                .await,
            Err(Error::Protocol(_))
        ),
        "malformed public data must not panic during a merge"
    );
    fixture.client.close().await.unwrap();
}

#[tokio::test]
async fn personal_reads_and_imports_cannot_return_the_previous_accounts_data() {
    for importing in [false, true] {
        let fixture = Fixture::start().await;
        let path = if importing {
            "/v1/loras/personal"
        } else {
            "/v1/loras/personal/personal-a"
        };
        fixture.reply(path, json!({"data":personal_entry()}));
        *fixture.state.hold_path.lock() = Some(path.into());
        let api = fixture.client.projects.personal_loras();
        let pending = tokio::spawn(async move {
            if importing {
                api.import(&ImportPersonalLoraParams {
                    url: "https://example.test/adapter.safetensors".into(),
                    name: "Adapter".into(),
                    model_id: "model-a".into(),
                    rights_confirmed: true,
                })
                .await
            } else {
                api.get("personal-a").await
            }
        });
        tokio::time::timeout(Duration::from_secs(10), fixture.state.started.notified())
            .await
            .unwrap();
        fixture.client.account.logout().await.unwrap();
        fixture.state.release.notify_one();
        let error = pending.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("account changed"), "{error}");
        fixture.client.close().await.unwrap();
    }
}

#[tokio::test]
async fn estimates_forward_billing_context_and_preserve_server_fair_use_percentage() {
    let fixture = Fixture::start().await;
    let image = "/api/v2/job/estimate/spark/fast/fixture-model/1/4/0/0/0/512/512";
    let video = "/api/v1/job-video/estimate/spark/ltx23-22b-fp8_t2v_distilled/512/512/121/24/8/1";
    let audio = "/api/v1/job-audio/estimate/spark/ace_step_1.5_turbo/30/8/1";
    for path in [image, video, audio] {
        fixture.reply(path, json!({"quote":{"project":{"costInToken":1.25}},"dailyFairUse":{"pct":0},"future":"preserved"}));
    }
    let image_result = fixture
        .client
        .projects
        .estimate_cost(&json!({
            "model":"fixture-model","imageCount":1,"stepCount":4,"previewCount":0,
            "width":512,"height":512,"billingMode":"auto"
        }))
        .await
        .unwrap();
    assert_eq!(image_result.daily_fair_use_pct, Some(0.0));
    assert_eq!(image_result.raw["future"], "preserved");
    fixture
        .client
        .projects
        .estimate_video_cost(&json!({
            "tokenType":"spark","model":"ltx23-22b-fp8_t2v_distilled","width":512,"height":512,
            "fps":24,"duration":5,"steps":8,"billingMode":"auto"
        }))
        .await
        .unwrap();
    fixture.client.projects.estimate_audio_cost(&json!({
        "tokenType":"spark","model":"ace_step_1.5_turbo","duration":30,"steps":8,"numberOfMedia":1,
        "network":"fast","billingMode":"tokens"
    })).await.unwrap();
    assert_eq!(fixture.requests(image)[0].query["billingMode"], "auto");
    assert_eq!(fixture.requests(video)[0].query["network"], "relaxed");
    assert_eq!(fixture.requests(audio)[0].query["network"], "fast");
    assert_eq!(fixture.requests(audio)[0].query["billingMode"], "tokens");
    fixture.reply(audio, json!({"quote":{"project":{"costInToken":1.25}}}));
    let quote = fixture.client.projects.estimate_audio_cost(&json!({
        "tokenType":"spark","model":"ace_step_1.5_turbo","duration":30,"steps":8,"numberOfMedia":1
    })).await.unwrap();
    assert_eq!(quote.daily_fair_use_pct, None);
    assert_eq!(fixture.requests(audio)[1].query["network"], "relaxed");
    assert!(!fixture.requests(audio)[1].query.contains_key("billingMode"));
    fixture.client.close().await.unwrap();
}
