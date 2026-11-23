use std::{sync::Arc, time::Duration};

use axum::{Json, Router, extract::State, http::Uri, routing::post};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{Error, SogniClient};

const ADDRESS: &str = "0x930fbddbfa47bf6263330d551414750c2688be5a";
const PASSWORD: &str = "correct horse battery staple";

#[derive(Clone)]
struct Fixture {
    approve: bool,
    refuse: bool,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

async fn handle(
    State(fixture): State<Fixture>,
    uri: Uri,
    Json(payload): Json<Value>,
) -> Json<Value> {
    fixture
        .calls
        .lock()
        .push((uri.path().into(), payload.clone()));
    if !uri.path().ends_with("/permit") {
        return Json(json!({"success":true}));
    }
    let mut data = json!({
        "domain":{"name":"Sogni AI","version":"1","chainId":"8453",
            "verifyingContract":"0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"},
        "types":{"WithdrawTokenV2":[
            {"name":"walletAddress","type":"address"}, {"name":"amount","type":"string"},
            {"name":"provider","type":"string"}, {"name":"nonce","type":"string"}
        ]}, "message":payload,
    });
    data["message"]["nonce"] = json!("synthetic-nonce");
    if fixture.approve {
        data["domain"]["verifyingContract"] = json!("0xE014d2A4da6E450f21b5050120D291e63c8940FD");
        data["types"] = json!({"Permit":[
            {"name":"owner","type":"address"}, {"name":"spender","type":"address"},
            {"name":"value","type":"uint256"}, {"name":"nonce","type":"uint256"},
            {"name":"deadline","type":"uint256"}
        ]});
        data["message"] =
            json!({"owner":ADDRESS,"spender":ADDRESS,"value":"100","nonce":"0","deadline":"0"});
    }
    if fixture.refuse {
        data["domain"]["chainId"] = json!("1");
    }
    Json(json!({"data":data}))
}

#[tokio::test]
async fn wallet_permits_are_checked_before_submission_and_keep_the_original_deadline() {
    for approve in [false, true] {
        for refuse in [false, true] {
            let fixture = Fixture {
                approve,
                refuse,
                calls: Arc::default(),
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/", listener.local_addr().unwrap())
                .parse()
                .unwrap();
            let app = Router::new()
                .fallback(post(handle))
                .with_state(fixture.clone());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let client = SogniClient::builder()
                .api_key("synthetic-permit-key")
                .disable_socket(true)
                .rest_endpoint(endpoint)
                .request_timeout(Duration::from_secs(3))
                .build()
                .await
                .unwrap();
            client
                .account
                .current
                .update(json!({"username":"TestUser","walletAddress":ADDRESS}));
            let result = if approve {
                client
                    .account
                    .approve_token_usage(PASSWORD, "account", "base")
                    .await
            } else {
                client.account.withdraw(PASSWORD, "1", "base").await
            };
            let calls = fixture.calls.lock().clone();
            if refuse {
                assert!(
                    matches!(result, Err(Error::Api(ref error)) if error.status == 400
                    && error.message == "Unexpected signing request; nothing was signed")
                );
                assert_eq!(calls.len(), 1, "refusal must never submit a signature");
            } else {
                result.unwrap();
                assert_eq!(calls.len(), 2);
                let field = if approve {
                    "approveSignature"
                } else {
                    "signature"
                };
                assert_eq!(calls[1].1[field].as_str().unwrap().len(), 132);
                assert_eq!(calls[1].1["walletAddress"], ADDRESS);
                if approve {
                    assert_eq!(calls[1].0, "/v1/contract/token/approve");
                    assert_eq!(calls[1].1["deadline"], "0");
                } else {
                    assert_eq!(calls[1].0, "/v2/account/token/withdraw");
                    assert_eq!(calls[1].1["amount"], "1000000000000000000");
                }
            }
            client.close().await.unwrap();
            server.abort();
        }
    }
}
