use super::*;

async fn destination() -> (TcpListener, Url) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/collect", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    (listener, url)
}

#[tokio::test]
async fn authenticated_posts_and_streams_never_follow_body_preserving_redirects() {
    for status in [
        "302 Found",
        "307 Temporary Redirect",
        "308 Permanent Redirect",
    ] {
        for streaming in [false, true] {
            let (destination, location) = destination().await;
            let (base, captured) = raw_http_fixture(
                status,
                |_| format!("Location: {location}\r\nContent-Type: application/json\r\n"),
                "{}",
            )
            .await;
            let rest = api_key_rest(base, Duration::from_secs(3));
            let error = if streaming {
                rest.stream_sse("/v1/chat/runs/fixture/events", None, HeaderMap::new())
                    .await
                    .err()
                    .unwrap()
            } else {
                rest.post(
                    "/v1/contract/token/approve/permit",
                    &json!({"walletAddress":"synthetic"}),
                )
                .await
                .unwrap_err()
            };
            assert!(
                matches!(error, Error::Api(ref error) if error.status == status[..3].parse::<u16>().unwrap())
            );
            let original = captured.await.unwrap().to_ascii_lowercase();
            assert!(original.contains("api-key: secret-test-key"));
            assert!(original.starts_with(if streaming {
                "get /v1/chat/runs/"
            } else {
                "post /v1/contract/"
            }));
            assert!(
                tokio::time::timeout(Duration::from_millis(100), destination.accept())
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn refresh_token_body_is_never_replayed_to_redirect_destination() {
    for status in [
        "302 Found",
        "307 Temporary Redirect",
        "308 Permanent Redirect",
    ] {
        let (destination, location) = destination().await;
        let (base, captured) = raw_http_fixture(
            status,
            |_| format!("Location: {location}\r\nContent-Type: application/json\r\n"),
            "{}",
        )
        .await;
        let http = HttpClients::build(Duration::from_secs(3)).unwrap();
        let auth = AuthManager::new(AuthKind::Token, base, http.authenticated(), http.cookies());
        let token = |expires| {
            format!(
                "e30.{}.fixture",
                b64_json_encode(&json!({"addr":"fixture","exp":expires})).unwrap()
            )
        };
        let error = auth
            .authenticate_tokens(token(1_u64), token(4_102_444_800_u64))
            .await
            .expect_err("refresh redirect refused");
        assert!(
            matches!(error, Error::Api(ref error) if error.status == status[..3].parse::<u16>().unwrap())
        );
        assert!(
            captured
                .await
                .unwrap()
                .starts_with("POST /v1/account/refresh-token ")
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), destination.accept())
                .await
                .is_err()
        );
    }
}
