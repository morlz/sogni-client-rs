use super::*;
use axum::{Json, Router, extract::Query, routing::get};
use std::{collections::HashMap, sync::Arc};

#[tokio::test]
async fn priority_history_is_opt_in_and_preserves_ordinary_amounts() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let queries = seen.clone();
    let router = Router::new().route(
        "/v1/transactions/list",
        get(move |Query(query): Query<HashMap<String, String>>| {
            queries.lock().push(query);
            async {
                Json(json!({"data":{"next":null,"transactions":[
                    {"id":"plan","amount":0,"priorityUsage":{"outputCount":1,"rate":1.5,
                        "coverage":"complete","unrecognized":"omit"}},
                    {"id":"ordinary","amount":2,"priorityUsage":{"outputCount":"invalid"}},
                ]}}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = crate::SogniClient::builder()
        .rest_endpoint(url)
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    for flag in [None, Some(false), Some(true)] {
        let mut query = json!({"provider":"all","limit":20});
        if let Some(flag) = flag {
            query["includePriorityAccounting"] = json!(flag);
        }
        let result = client.account.transaction_history(&query).await.unwrap();
        let plan = &result["transactions"][0];
        assert_eq!(plan["amount"], 0);
        if flag == Some(true) {
            assert_eq!(
                plan["priorityUsage"],
                json!({"outputCount":1,"rate":1.5,"coverage":"complete"})
            );
        } else {
            assert!(plan.get("priorityUsage").is_none());
        }
        assert!(result["transactions"][1].get("priorityUsage").is_none());
    }
    let queries = seen.lock().clone();
    for query in &queries[..2] {
        assert!(!query.contains_key("includePriorityAccounting"));
    }
    assert_eq!(queries[2]["includePriorityAccounting"], "true");
    assert_eq!(queries[2]["limit"], "20");
    client.close().await.unwrap();
    server.abort();
}
