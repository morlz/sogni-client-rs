use std::sync::Arc;

use axum::{Json, Router, routing::get};
use serde_json::json;

use crate::SogniClient;

#[tokio::test]
async fn cookie_identity_discovery_clears_previous_account_projection() {
    for check_auth in [false, true] {
        let identity = Arc::new(parking_lot::Mutex::new("cookie-a"));
        let response_identity = identity.clone();
        let router = Router::new().route(
            "/v1/account/me",
            get(move || {
                let identity = response_identity.clone();
                async move {
                    let address = *identity.lock();
                    Json(json!({"data":{"username":address,"walletAddress":address}}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client = SogniClient::builder()
            .cookie_auth()
            .disable_socket(true)
            .rest_endpoint(endpoint.parse().unwrap())
            .build()
            .await
            .unwrap();
        assert!(client.check_auth().await.unwrap());
        client.account.current.update(json!({
            "balance":{"sogni":"account-a"},
            "subscription":{"active":true,"tier":"unlimited"},
            "freeSparkLocked":true,"freeSparkUnlockPath":"purchase"
        }));
        {
            let mut projection = client.account.subscription_projection.lock();
            projection.last_version = Some(99.0);
            projection.socket_writes = 7;
        }

        // Rediscovering the same cookie owner must retain its live projection.
        if check_auth {
            assert!(client.check_auth().await.unwrap());
        } else {
            client.account.me().await.unwrap();
        }
        assert_eq!(client.current_account().balance()["sogni"], "account-a");
        assert!(client.current_account().is_unlimited());

        *identity.lock() = "cookie-b";
        if check_auth {
            assert!(client.check_auth().await.unwrap());
        } else {
            client.account.me().await.unwrap();
        }
        assert_eq!(
            client.current_account().wallet_address().as_deref(),
            Some("cookie-b")
        );
        assert_ne!(client.current_account().balance()["sogni"], "account-a");
        assert!(client.current_account().subscription().is_none());
        assert_eq!(client.current_account().free_spark_locked(), None);
        assert_eq!(client.current_account().free_spark_unlock_path(), None);
        {
            let projection = client.account.subscription_projection.lock();
            assert_eq!(projection.last_version, None);
            assert_eq!(projection.socket_writes, 0);
        }
        client.close().await.unwrap();
        server.abort();
    }
}
