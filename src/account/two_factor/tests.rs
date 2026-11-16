use super::*;
use crate::SogniClient;
use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode, header},
    routing::{get, post},
};

const CHALLENGE: &str = "synthetic-sign-in-challenge";
const TOKEN: &str = "eyJhbGciOiJub25lIn0.eyJleHAiOjQxMDI0NDQ4MDB9.signature";

#[tokio::test]
async fn challenge_stages_neither_tokens_nor_cookies_and_allows_a_second_proof() {
    let app = Router::new()
        .route(
            "/v1/account/nonce",
            post(|| async { Json(json!({"data":{"nonce":"fixture"}})) }),
        )
        .route(
            "/v1/account/login",
            post(|| async {
                (
                    [(header::SET_COOKIE, "fixture=unverified; Path=/")],
                    Json(json!({"data":{
                        "requires2FA":true,"tempToken":CHALLENGE,"username":"TestUser",
                        "token":TOKEN,"refreshToken":TOKEN
                    }})),
                )
            }),
        )
        .route(
            "/v1/account/2fa/passkeys/login-options",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert!(!headers.contains_key(header::COOKIE));
                assert!(!headers.contains_key(header::AUTHORIZATION));
                assert_eq!(body, json!({"tempToken":CHALLENGE}));
                Json(json!({"data":{"challengeId":"pk","options":{"challenge":"synthetic"}}}))
            }),
        )
        .route(
            "/v1/account/2fa/verify-login",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["tempToken"], CHALLENGE);
                if body.get("code").is_some() {
                    assert_eq!(body["code"], "FAILURE");
                    (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"errorCode":401,"message":"Incorrect code"})),
                    )
                } else {
                    assert_eq!(body["challengeId"], "pk");
                    assert_eq!(body["assertion"]["response"], json!({"userHandle":null}));
                    (
                        StatusCode::OK,
                        Json(json!({"data":{"token":TOKEN,"refreshToken":TOKEN}})),
                    )
                }
            }),
        )
        .route(
            "/v1/account/me",
            get(|headers: HeaderMap| async move {
                assert!(headers.contains_key(header::AUTHORIZATION));
                Json(json!({"data":{"username":"TestUser"}}))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let new_client = || {
        SogniClient::builder()
            .disable_socket(true)
            .rest_endpoint(base.parse().unwrap())
            .build()
    };
    let client = new_client().await.unwrap();
    let other = new_client().await.unwrap();
    let error = client
        .account
        .login("TestUser", "fixture-password")
        .await
        .unwrap_err();
    assert!(!format!("{error:?} {error}").contains(CHALLENGE));
    let Error::TwoFactorRequired(challenge) = error else {
        panic!("expected the typed challenge");
    };
    assert!(!client.is_authenticated());
    assert!(client.account.current_account().username().is_none());
    assert!(matches!(
        other
            .account
            .get_two_factor_passkey_options(Some(&challenge))
            .await,
        Err(Error::InvalidInput(_))
    ));
    let first = client
        .account
        .verify_two_factor_login(&challenge, &TwoFactorProof::Code(" failure ".into()))
        .await;
    assert!(matches!(first, Err(Error::Api(error)) if error.status == 401));
    let options = client
        .account
        .get_two_factor_passkey_options(Some(&challenge))
        .await
        .unwrap();
    let result = client
        .account
        .verify_two_factor_login(
            &challenge,
            &TwoFactorProof::Passkey {
                assertion: json!({"response":{"userHandle":null}}),
                challenge_id: options.challenge_id,
            },
        )
        .await
        .unwrap();
    assert_eq!(result["username"], "TestUser");
    assert!(result.get("tempToken").is_none());
    assert_eq!(
        client.account.current_account().username().as_deref(),
        Some("TestUser")
    );
    assert!(client.is_authenticated());
    assert!(matches!(
        client
            .account
            .get_two_factor_passkey_options(Some(&challenge))
            .await,
        Err(Error::InvalidInput(_))
    ));
    client.close().await.unwrap();
    other.close().await.unwrap();
    server.abort();
}
