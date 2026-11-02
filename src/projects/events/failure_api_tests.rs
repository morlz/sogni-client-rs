use super::*;
use axum::{Json, Router, extract::State, routing::get};

#[tokio::test]
async fn project_failure_5_72_matches_owner_history_without_private_details() {
    let fixture: Value =
        serde_json::from_str(include_str!("../wire/fixtures/upstream-5-72.json")).unwrap();
    let state = Arc::new(parking_lot::Mutex::new(Value::Null));
    let router = Router::new()
        .route(
            "/v2/projects/P",
            get(
                |State(snapshot): State<Arc<parking_lot::Mutex<Value>>>| async move {
                    let project = snapshot.lock().clone();
                    Json(json!({"data":{"project":project}}))
                },
            ),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = crate::SogniClient::builder()
        .app_id("failure-history-fixture")
        .api_key("synthetic-fixture")
        .defer_socket_start(true)
        .rest_endpoint(format!("http://{address}").parse().unwrap())
        .socket_endpoint(format!("ws://{address}").parse().unwrap())
        .build()
        .await
        .unwrap();
    for case in fixture["failures"].as_array().unwrap() {
        *state.lock() = case["snapshot"].clone();
        let result = client.projects.get_result("P", None).await.unwrap();
        assert!(result.jobs.is_empty());
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
    }
    client.close().await.unwrap();
    server.abort();
    let _ = server.await;
}
