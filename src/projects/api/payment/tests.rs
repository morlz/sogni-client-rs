use super::*;
use crate::{
    SogniClient,
    utils::{b64_json_decode, b64_json_encode},
};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    WebSocketStream, accept_async,
    tungstenite::{
        Message,
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

async fn connect(listener: &TcpListener) -> WebSocketStream<TcpStream> {
    loop {
        // Project recovery also uses HTTP on the socket endpoint.
        if let Ok(socket) = accept_async(listener.accept().await.unwrap().0).await {
            return socket;
        }
    }
}

async fn event(ws: &mut WebSocketStream<TcpStream>, name: &str, data: Value) {
    ws.send(Message::Text(
        json!({"type":name,"data":b64_json_encode(&data).unwrap()})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
}

async fn expect_switch(ws: &mut WebSocketStream<TcpStream>) {
    loop {
        if let Message::Text(text) = ws.next().await.unwrap().unwrap() {
            let envelope: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(envelope["type"], "switchProjectToPremiumSpark");
            let request = b64_json_decode(envelope["data"].as_str().unwrap()).unwrap();
            assert_eq!(request, json!({"projectId":"Fixture"}));
            return;
        }
    }
}

#[tokio::test]
async fn concurrent_switches_share_one_write_per_connection_and_preserve_unknown_after_reconnect() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let rest = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rest_url = format!("http://{}", rest.local_addr().unwrap())
            .parse()
            .unwrap();
        let http = tokio::spawn(async move {
            let router = axum::Router::new().route(
                "/v1/account/me",
                axum::routing::get(|| async { axum::Json(json!({"data":{"username":"fixture"}})) }),
            );
            axum::serve(rest, router).await.unwrap();
        });
        let server =
            tokio::spawn(async move {
                let mut first = connect(&listener).await;
                event(&mut first, "authenticated", json!({})).await;
                expect_switch(&mut first).await;
                first
                    .close(Some(CloseFrame {
                        code: CloseCode::Away,
                        reason: "fixture reconnect".into(),
                    }))
                    .await
                    .unwrap();
                let mut second = connect(&listener).await;
                event(&mut second, "authenticated", json!({})).await;
                expect_switch(&mut second).await;
                event(
                    &mut second,
                    "projectPaymentSwitch",
                    json!({"jobID":"other","status":"switched"}),
                )
                .await;
                event(&mut second, "projectPaymentSwitch", json!({
                "jobID":"fIxTuRe","status":"refused","reason":"not_found","costInSpark":"1.25"
            })).await;
                while let Some(Ok(message)) = second.next().await {
                    assert!(!message.is_text(), "a settled request must not be resent");
                }
            });
        let client = SogniClient::builder()
            .app_id("payment-fixture")
            .api_key("fixture-key")
            .rest_endpoint(rest_url)
            .socket_endpoint(endpoint)
            .build()
            .await
            .unwrap();
        let a = client.projects.switch_to_premium_spark("Fixture", None);
        let b = client.projects.switch_to_premium_spark("fixture", None);
        let (a, b) = tokio::join!(a, b);
        let a = a.unwrap();
        assert_eq!(a, b.unwrap());
        assert_eq!(a.project_id, "Fixture");
        assert_eq!(a.cost_in_spark.as_deref(), Some("1.25"), "{a:?}");
        assert_eq!(
            a.outcome,
            PayAsYouGoSwitchOutcome::Unknown {
                reason: PayAsYouGoUnknownReason::NotFound
            }
        );
        client.close().await.unwrap();
        server.await.unwrap();
        http.abort();
    })
    .await
    .expect("the payment fixture must complete within the watchdog");
}

#[tokio::test]
async fn disabled_socket_reports_a_definitely_unsent_request() {
    let client = SogniClient::builder()
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    let result = client
        .projects
        .switch_to_premium_spark("Fixture", None)
        .await
        .unwrap();
    assert_eq!(
        result.outcome,
        PayAsYouGoSwitchOutcome::Refused {
            reason: PayAsYouGoRefusalReason::NotSent
        }
    );
    client.close().await.unwrap();
}
