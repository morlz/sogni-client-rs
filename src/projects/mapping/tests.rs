use std::time::Duration;

use axum::{Json, Router, routing::get};

use super::*;
use crate::SogniClient;

// Public model/tier fields captured through the read-only catalogue on 2026-09-05.
// Neither model records nor available-model projections carry this capability.
fn fixture() -> Value {
    serde_json::from_str(include_str!("rtx-options-fixture.json")).unwrap()
}

#[tokio::test]
async fn native_tier_capability_survives_the_public_model_options_path() {
    let captured = fixture();
    assert!(captured["model"].get("isUpscale").is_none());
    assert_eq!(captured["tier"]["isUpscale"], true);
    let model = captured["model"].clone();
    let tier_id = model["tier"].as_str().unwrap().to_owned();
    let sid = model["SID"].as_i64().unwrap().to_string();
    let tiers = json!({tier_id: captured["tier"]});
    let workers = json!({sid: 1});
    let app = Router::new()
        .route(
            "/api/v1/models/list",
            get(move || async move { Json(json!([model])) }),
        )
        .route(
            "/api/v2/models/tiers",
            get(move || async move { Json(tiers) }),
        )
        .route(
            "/api/v1/status/network/fast/models",
            get(move || async move { Json(workers) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Url::parse(&format!("ws://{}/", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = SogniClient::builder()
        .api_key("local-fixture")
        .socket_endpoint(endpoint)
        .defer_socket_start(true)
        .request_timeout(Duration::from_secs(2))
        .build()
        .await
        .unwrap();
    let available = client
        .projects
        .get_available_models(Network::Fast)
        .await
        .unwrap();
    assert_eq!(available.len(), 1);
    assert!(available[0].get("isUpscale").is_none());
    let options = client
        .projects
        .get_model_options("rtx_vsr_pro", false)
        .await
        .unwrap();
    assert_eq!(options.model_id, "rtx_vsr_pro");
    assert_eq!(options.media_type, "image");
    assert_eq!(options.raw["isUpscale"], true);
    for name in ["steps", "width", "height"] {
        for bound in ["min", "max", "default"] {
            assert_eq!(options.raw[name][bound], captured["tier"][name][bound]);
        }
    }
    assert_eq!(options.raw["width"]["step"], 8.0);
    assert!(!client.is_socket_connected());
    client.close().await.unwrap();
    server.abort();
}

#[test]
fn native_tier_capability_does_not_coerce_missing_or_malformed_values() {
    let mut tier = fixture()["tier"].clone();
    tier["isUpscale"] = json!(false);
    assert_eq!(map_model_options(&tier, "image")["isUpscale"], false);
    for invalid in [
        Value::Null,
        json!("true"),
        json!(1),
        json!({"default": true}),
    ] {
        tier["isUpscale"] = invalid;
        assert!(map_model_options(&tier, "image").get("isUpscale").is_none());
    }
    tier.as_object_mut().unwrap().remove("isUpscale");
    assert!(map_model_options(&tier, "image").get("isUpscale").is_none());
}
