#[cfg(test)]
mod tests {
    use axum::{
        Json, Router,
        http::{HeaderMap, HeaderValue},
        response::{IntoResponse, Sse, sse::Event},
        routing::{get, post},
    };
    use base64::{
        Engine,
        engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    };
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use sogni_client::{AuthBackup, SogniClient};
    use std::{
        collections::BTreeMap,
        convert::Infallible,
        sync::Arc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tokio::{
        net::TcpListener,
        sync::{Notify, mpsc},
        task::JoinHandle,
    };
    use tokio_tungstenite::{
        accept_hdr_async,
        tungstenite::{
            Message,
            handshake::server::{Request, Response},
        },
    };

    // Loopback fixtures still share CPU with compilation and parallel CI jobs.
    // These bounds detect a stuck operation without competing with SDK deadlines.
    const WATCHDOG: Duration = Duration::from_secs(10);
    const STREAM_TIMEOUT: Duration = Duration::from_secs(1);

    fn now() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    }
    fn token(user: &str, exp: f64) -> String {
        format!(
            "e30.{}.fixture",
            URL_SAFE_NO_PAD.encode(json!({"sub":user,"exp":exp}).to_string())
        )
    }
    fn account(headers: HeaderMap) -> Json<Value> {
        if !headers.contains_key("authorization") && !headers.contains_key("api-key") {
            assert!(
                headers
                    .get("cookie")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .contains("review-session=fixture"),
                "REST must have received the login cookie before checking its WebSocket transfer"
            );
        }
        let user = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split('.').nth(1))
            .and_then(|v| URL_SAFE_NO_PAD.decode(v).ok())
            .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
            .and_then(|v| v["sub"].as_str().map(str::to_owned))
            .unwrap_or("cookie-user".into());
        Json(json!({"status":"success","data":{"username":user,"email":"fixture@example.test"}}))
    }
    fn base_router() -> Router {
        Router::new().route("/v1/account/me", get(|h: HeaderMap| async move { account(h) }))
            .route("/v1/account/nonce", post(|| async { Json(json!({"data":{"nonce":"review-nonce"}})) }))
            .route("/v1/account/login", post(|| async {
                let mut headers = HeaderMap::new();
                headers.insert("set-cookie", HeaderValue::from_static("review-session=fixture; Path=/; HttpOnly"));
                (headers, Json(json!({"status":"success","data":{}})))
            }))
            .fallback(|| async { Json(json!({"status":"success","data":{},"activeProjects":[],"unclaimedCompletedProjects":[]})) })
    }
    async fn http(router: Router) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        (
            url,
            tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            }),
        )
    }
    #[derive(Debug)]
    struct Wire {
        authorization: String,
        cookie: String,
        message: String,
    }
    // Tungstenite fixes the handshake callback's HTTP response error type.
    #[allow(clippy::result_large_err)]
    async fn socket() -> (String, mpsc::UnboundedReceiver<Wire>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let auth = Arc::new(std::sync::Mutex::new((String::new(), String::new())));
                    let capture = auth.clone();
                    let ws = accept_hdr_async(stream, move |r: &Request, response: Response| {
                        let read = |name: &str| {
                            r.headers()
                                .get(name)
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("")
                                .to_owned()
                        };
                        *capture.lock().unwrap() = (read("authorization"), read("cookie"));
                        Ok(response)
                    })
                    .await;
                    let Ok(mut ws) = ws else {
                        return;
                    };
                    let (authorization, cookie) = auth.lock().unwrap().clone();
                    let _ = tx.send(Wire {
                        authorization: authorization.clone(),
                        cookie: cookie.clone(),
                        message: "handshake".into(),
                    });
                    let frame =
                        json!({"type":"authenticated","data":STANDARD.encode("{}")}).to_string();
                    if ws.send(Message::Text(frame.into())).await.is_err() {
                        return;
                    }
                    while let Some(Ok(message)) = ws.next().await {
                        match message {
                            Message::Text(message) => {
                                let _ = tx.send(Wire {
                                    authorization: authorization.clone(),
                                    cookie: cookie.clone(),
                                    message: message.to_string(),
                                });
                            }
                            Message::Ping(data) => {
                                let _ = ws.send(Message::Pong(data)).await;
                            }
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                });
            }
        });
        (url, rx, task)
    }
    async fn authenticated(client: &SogniClient) {
        tokio::time::timeout(WATCHDOG, async {
            while !client.is_socket_authenticated() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the fixture socket should authenticate");
    }

    #[tokio::test]
    async fn account_switch_must_reauthenticate_the_socket() {
        let (rest, rest_task) = http(base_router()).await;
        let (ws, mut wire, ws_task) = socket().await;
        let client = SogniClient::builder()
            .app_id("review-switch")
            .rest_endpoint(rest.parse().unwrap())
            .socket_endpoint(ws.parse().unwrap())
            .build()
            .await
            .unwrap();
        let first = token("account-a", now() + 3600.0);
        let second = token("account-b", now() + 3600.0);
        client
            .set_tokens(first.clone(), token("refresh-a", now() + 7200.0))
            .await
            .unwrap();
        authenticated(&client).await;
        let handshake = tokio::time::timeout(WATCHDOG, wire.recv())
            .await
            .expect("the first account should complete its socket handshake")
            .unwrap();
        assert_eq!(handshake.authorization, first);
        assert!(handshake.cookie.is_empty());
        client
            .set_tokens(second.clone(), token("refresh-b", now() + 7200.0))
            .await
            .unwrap();
        client
            .set_socket_event_subscriptions(BTreeMap::from([("jobState".into(), true)]))
            .await
            .unwrap();
        let sent = tokio::time::timeout(WATCHDOG, async {
            loop {
                let event = wire.recv().await.unwrap();
                if event.message != "handshake" {
                    break event;
                }
            }
        })
        .await
        .expect("the new account should receive the queued socket command");
        let backup = client.auth_backup().unwrap();
        assert!(matches!(backup,Some(AuthBackup::Tokens{token,..}) if token==second));
        client.abort();
        rest_task.abort();
        ws_task.abort();
        assert_eq!(
            sent.authorization, second,
            "REST has account B, but the socket command still uses account A's connection"
        );
    }

    #[cfg(feature = "wallet")]
    #[tokio::test]
    async fn cookie_login_must_send_the_cookie_on_websocket_upgrade() {
        let (rest, rest_task) = http(base_router()).await;
        let (ws, mut wire, ws_task) = socket().await;
        let client = SogniClient::builder()
            .app_id("review-cookie")
            .cookie_auth()
            .rest_endpoint(rest.parse().unwrap())
            .socket_endpoint(ws.parse().unwrap())
            .build()
            .await
            .unwrap();
        client
            .account
            .login("sdk-review-local-fixture", "dummy-password")
            .await
            .unwrap();
        let handshake = tokio::time::timeout(WATCHDOG, wire.recv())
            .await
            .unwrap()
            .unwrap();
        client.abort();
        rest_task.abort();
        ws_task.abort();
        assert!(
            handshake.cookie.contains("review-session=fixture"),
            "cookie login succeeded, but the WebSocket Cookie header was {:?}",
            handshake.cookie
        );
    }

    #[tokio::test]
    async fn healthy_sse_must_outlive_the_regular_request_timeout() {
        let router = base_router().route("/v1/creative-agent/workflows/review/events/stream", get(|| async {
            Sse::new(async_stream::stream! {
                for i in 0..100 {
                    yield Ok::<_,Infallible>(Event::default().id(i.to_string()).data("{\"status\":\"running\"}"));
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }).into_response()
        }));
        let (rest, task) = http(router).await;
        let client = SogniClient::builder()
            .disable_socket(true)
            .api_key("local-fixture")
            .rest_endpoint(rest.parse().unwrap())
            .request_timeout(STREAM_TIMEOUT)
            .build()
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let mut events = client
            .workflows
            .stream_events("review", None, None)
            .await
            .unwrap();
        let mut seen = 0;
        let mut error = None;
        tokio::time::timeout(WATCHDOG, async {
            for _ in 0..20 {
                match events.next().await {
                    Some(Ok(_)) => seen += 1,
                    Some(Err(e)) => {
                        error = Some(e);
                        break;
                    }
                    None => break,
                }
            }
        })
        .await
        .expect("the healthy SSE fixture should continue yielding events");
        client.abort();
        task.abort();
        assert_eq!(
            seen,
            20,
            "healthy SSE was cut off after {:?}: {:?}",
            started.elapsed(),
            error
        );
        assert!(started.elapsed() > STREAM_TIMEOUT);
    }

    #[tokio::test]
    async fn in_flight_refresh_must_not_replace_a_newer_account() {
        stale_refresh(false, false).await;
    }

    #[tokio::test]
    async fn failed_refresh_must_not_clear_a_newer_account() {
        stale_refresh(true, false).await;
    }

    #[tokio::test]
    async fn in_flight_refresh_must_not_restore_a_closed_session() {
        stale_refresh(false, true).await;
    }

    #[tokio::test]
    async fn refresh_waiter_must_keep_its_original_account_session() {
        for switch_account in [false, true] {
            let begun = Arc::new(Notify::new());
            let release = Arc::new(Notify::new());
            let refreshed = token("account-a-refreshed", now() + 3600.0);
            let response_token = refreshed.clone();
            let begun_handler = begun.clone();
            let release_handler = release.clone();
            let (mutations, mut received) = mpsc::unbounded_channel();
            let router = base_router()
                .route(
                    "/v1/account/refresh-token",
                    post(move || {
                        let begun = begun_handler.clone();
                        let release = release_handler.clone();
                        let response_token = response_token.clone();
                        async move {
                            begun.notify_one();
                            release.notified().await;
                            Json(json!({"data": {
                                "token": response_token,
                                "refreshToken": token("refresh-a2", now() + 7200.0)
                            }}))
                        }
                    }),
                )
                .route(
                    "/v1/account/device-id",
                    post(move |headers: HeaderMap, Json(body): Json<Value>| {
                        let mutations = mutations.clone();
                        async move {
                            mutations
                                .send((headers["authorization"].to_str().unwrap().to_owned(), body))
                                .unwrap();
                            Json(json!({"status": "success", "data": {}}))
                        }
                    }),
                );
            let (rest, task) = http(router).await;
            let client = SogniClient::builder()
                .disable_socket(true)
                .rest_endpoint(rest.parse().unwrap())
                .build()
                .await
                .unwrap();
            let expires_at = now() + 2.0;
            client
                .set_tokens(
                    token("account-a", expires_at),
                    token("refresh-a", now() + 7200.0),
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs_f64(
                (expires_at - now()).max(0.0) + 0.01,
            ))
            .await;

            let account_api = client.account.clone();
            let refreshing = tokio::spawn(async move { account_api.account_balance().await });
            tokio::time::timeout(WATCHDOG, begun.notified())
                .await
                .expect("the first request must hold the refresh lock");
            let waiting = client.account.set_device_id("device-from-account-a");
            tokio::pin!(waiting);
            // Poll the entire public request path into the held refresh lock.
            // The request now originates in A's session before any switch to B.
            assert!(futures_util::poll!(waiting.as_mut()).is_pending());
            let second = token("account-b", now() + 3600.0);
            if switch_account {
                tokio::time::timeout(
                    WATCHDOG,
                    client.set_tokens(second.clone(), token("refresh-b", now() + 7200.0)),
                )
                .await
                .expect("account B must install while account A's refresh is pending")
                .unwrap();
            }
            release.notify_one();
            let refreshing = tokio::time::timeout(WATCHDOG, refreshing)
                .await
                .expect("the released refresh must finish")
                .unwrap();
            let waiting = tokio::time::timeout(WATCHDOG, waiting)
                .await
                .expect("the request waiting for refresh must finish");
            let mutation = received.try_recv().ok();
            let backup = client.auth_backup().unwrap();
            client.abort();
            task.abort();

            if switch_account {
                assert!(refreshing.is_err());
                assert!(
                    mutation.is_none(),
                    "an account-A request reached the service after switching to account B: {mutation:?}"
                );
                assert!(
                    matches!(waiting, Err(sogni_client::Error::InvalidInput(message))
                    if message.contains("account session changed"))
                );
                assert!(
                    matches!(backup, Some(AuthBackup::Tokens { token, .. }) if token == second)
                );
            } else {
                refreshing.unwrap();
                waiting
                    .expect("a normal token revision must preserve the request's account session");
                let (authorization, body) =
                    mutation.expect("the same-account mutation must be sent");
                assert_eq!(authorization, refreshed);
                assert_eq!(body["deviceId"], "device-from-account-a");
            }
        }
    }

    async fn stale_refresh(fail: bool, close: bool) {
        let begun = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let expired_account_token = token("account-a-refreshed", now() + 3600.0);
        let response_token = expired_account_token.clone();
        let begun_handler = begun.clone();
        let release_handler = release.clone();
        let router=base_router().route("/v1/account/refresh-token",post(move || {
            let begun=begun_handler.clone(); let release=release_handler.clone(); let token=response_token.clone();
            async move {
                begun.notify_one(); release.notified().await;
                let status = if fail { axum::http::StatusCode::UNAUTHORIZED } else { axum::http::StatusCode::OK };
                (status, Json(json!({"data":{"token":token,"refreshToken":super::tests::token("refresh-a2",now()+7200.0)}})))
            }
        }));
        let (rest, task) = http(router).await;
        let client = SogniClient::builder()
            .disable_socket(true)
            .rest_endpoint(rest.parse().unwrap())
            .build()
            .await
            .unwrap();
        // Give initial hydration enough time to finish, then wait for the actual
        // JWT expiry before deliberately starting the blocked refresh request.
        let expires_at = now() + 2.0;
        client
            .set_tokens(
                token("account-a", expires_at),
                token("refresh-a", now() + 7200.0),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs_f64(
            (expires_at - now()).max(0.0) + 0.01,
        ))
        .await;
        let account_api = client.account.clone();
        let request = tokio::spawn(async move { account_api.account_balance().await });
        tokio::time::timeout(WATCHDOG, begun.notified())
            .await
            .unwrap();
        let second = token("account-b", now() + 3600.0);
        if close {
            client.close().await.unwrap();
        } else {
            client
                .set_tokens(second.clone(), token("refresh-b", now() + 7200.0))
                .await
                .unwrap();
        }
        release.notify_one();
        assert!(
            tokio::time::timeout(WATCHDOG, request)
                .await
                .expect("the released stale refresh should finish")
                .unwrap()
                .is_err()
        );
        if close {
            assert!(client.auth_backup().unwrap().is_none());
            task.abort();
            return;
        }
        let actual = match client.auth_backup().unwrap().unwrap() {
            AuthBackup::Tokens { token, .. } => token,
            _ => panic!("wrong auth kind"),
        };
        client.abort();
        task.abort();
        assert_eq!(
            actual, second,
            "a stale refresh response restored the previous account's credentials"
        );
    }

    #[tokio::test]
    async fn idle_sse_must_time_out() {
        let router = base_router().route(
            "/v1/creative-agent/workflows/idle/events/stream",
            get(|| async {
                Sse::new(async_stream::stream! {
                    yield Ok::<_, Infallible>(Event::default().data("{}"));
                    std::future::pending::<()>().await;
                })
            }),
        );
        let (rest, task) = http(router).await;
        let client = SogniClient::builder()
            .disable_socket(true)
            .api_key("local-fixture")
            .rest_endpoint(rest.parse().unwrap())
            .request_timeout(STREAM_TIMEOUT)
            .build()
            .await
            .unwrap();
        let mut events = client
            .workflows
            .stream_events("idle", None, None)
            .await
            .unwrap();
        assert!(events.next().await.unwrap().is_ok());
        let result = tokio::time::timeout(WATCHDOG, events.next()).await.unwrap();
        assert!(result.unwrap().is_err());
        client.abort();
        task.abort();
    }

    #[tokio::test]
    async fn late_unauthorized_response_must_not_clear_a_newer_account() {
        let begun = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let wait = release.clone();
        let signal = begun.clone();
        let router = base_router().route(
            "/v4/account/balance",
            get(move || {
                let signal = signal.clone();
                let wait = wait.clone();
                async move {
                    signal.notify_one();
                    wait.notified().await;
                    axum::http::StatusCode::UNAUTHORIZED
                }
            }),
        );
        let (rest, task) = http(router).await;
        let client = SogniClient::builder()
            .disable_socket(true)
            .rest_endpoint(rest.parse().unwrap())
            .build()
            .await
            .unwrap();
        client
            .set_tokens(token("a", now() + 3600.0), token("ra", now() + 7200.0))
            .await
            .unwrap();
        let account = client.account.clone();
        let request = tokio::spawn(async move { account.account_balance().await });
        tokio::time::timeout(WATCHDOG, begun.notified())
            .await
            .unwrap();
        let second = token("b", now() + 3600.0);
        client
            .set_tokens(second.clone(), token("rb", now() + 7200.0))
            .await
            .unwrap();
        release.notify_one();
        assert!(
            tokio::time::timeout(WATCHDOG, request)
                .await
                .expect("the released stale unauthorized response should finish")
                .unwrap()
                .is_err()
        );
        assert!(
            matches!(client.auth_backup().unwrap(), Some(AuthBackup::Tokens { token, .. }) if token == second)
        );
        client.abort();
        task.abort();
    }
}
