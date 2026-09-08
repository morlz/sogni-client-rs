use super::*;
use crate::Error;

#[tokio::test]
async fn rest_only_client_needs_no_app_id_and_never_connects_a_socket() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = SogniClient::builder()
        .api_key("local-fixture")
        .disable_socket(true)
        .socket_endpoint(Url::parse(&format!("ws://{address}/")).unwrap())
        .build()
        .await
        .unwrap();
    assert!(format!("{client:?}").contains("rest-only"));
    assert!(client.is_authenticated());
    assert!(!client.is_socket_connected());
    let error = client
        .chat
        .create_completion(&json!({
            "model":"fixture", "messages":[{"role":"user","content":"hello"}]
        }))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::InvalidInput(_)));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn socket_clients_require_and_trim_stable_installation_ids() {
    for id in ["", "  "] {
        let error = SogniClient::builder().app_id(id).build().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("appId is required when WebSocket connections are enabled")
        );
    }
    let client = SogniClient::builder()
        .app_id(" stable-app ")
        .build()
        .await
        .unwrap();
    assert!(format!("{client:?}").contains("\"stable-app\""));
    client.close().await.unwrap();
    let client = SogniClient::builder()
        .app_id("  ")
        .disable_socket(true)
        .build()
        .await
        .unwrap();
    assert!(format!("{client:?}").contains("rest-only"));
    client.close().await.unwrap();
}
