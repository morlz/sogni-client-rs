use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use tokio::{sync::Notify, task::JoinHandle};

use super::*;

const WATCHDOG: Duration = Duration::from_secs(5);

fn token(address: &str, serial: u32, expired: bool) -> String {
    format!(
        "e30.{}.fixture",
        URL_SAFE_NO_PAD.encode(
            json!({
                "addr":address, "serial":serial, "exp":if expired { 1 } else { 4_102_444_800_u64 }
            })
            .to_string()
        )
    )
}

fn rest_and_auth(url: Url) -> (RestClient, AuthManager) {
    let http = HttpClients::build(Duration::from_secs(30)).unwrap();
    let auth = api_key_auth(url.clone(), &http);
    (
        RestClient::new(url, auth.clone(), http, Duration::from_secs(30)),
        auth,
    )
}

fn assert_changed<T>(result: crate::Result<T>) {
    assert!(
        matches!(result, Err(Error::InvalidInput(ref message)) if message.contains("account session changed"))
    );
}

// The response never advances until the caller releases it. Cancellation must
// therefore finish locally, independently of a late server header or body.
async fn held_response(
    status: &str,
    content_type: &str,
    initial: &str,
    remaining: &str,
    hold_headers: bool,
) -> (
    Url,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: Url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        initial.len() + remaining.len()
    );
    let initial = initial.to_owned();
    let remaining = remaining.to_owned();
    let (entered, started) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            if read == 0 {
                return;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
        }
        let _ = entered.send(());
        if hold_headers {
            let _ = released.await;
            let _ = stream
                .write_all(format!("{header}{initial}{remaining}").as_bytes())
                .await;
        } else {
            if stream
                .write_all(format!("{header}{initial}").as_bytes())
                .await
                .is_err()
            {
                return;
            }
            let _ = released.await;
            let _ = stream.write_all(remaining.as_bytes()).await;
        }
    });
    (url, started, release, server)
}

#[tokio::test]
async fn late_rest_headers_and_exact_post_cannot_escape_their_account_session() {
    for (status, exact) in [
        ("200 OK", false),
        ("401 Unauthorized", false),
        ("200 OK", true),
    ] {
        let (url, started, release, server) =
            held_response(status, "application/json", "", "{}", true).await;
        let (rest, auth) = rest_and_auth(url);
        let request = tokio::spawn(async move {
            if exact {
                rest.post_exact_with("/preview", &json!({"keep":null}), HeaderMap::new())
                    .await
            } else {
                rest.get("/account", None).await
            }
        });
        started.await.unwrap();
        auth.authenticate_api_key("replacement-key").unwrap();
        assert_changed(
            tokio::time::timeout(WATCHDOG, request)
                .await
                .unwrap()
                .unwrap(),
        );
        assert!(
            matches!(auth.backup().unwrap(), Some(crate::AuthBackup::ApiKey(key)) if key == "replacement-key")
        );
        let _ = release.send(());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn raw_response_body_stays_bound_even_after_matching_401_clears_credentials() {
    for status in ["200 OK", "401 Unauthorized"] {
        let (url, _, release, server) =
            held_response(status, "application/json", "", "{\"data\":\"old\"}", false).await;
        let (rest, auth) = rest_and_auth(url);
        let response = rest
            .raw_request(reqwest::Method::GET, "/account", None, None, None, None)
            .await
            .unwrap();
        assert_eq!(auth.is_authenticated(), status == "200 OK");
        let processing = tokio::spawn(async move { rest.process_response(response).await });
        auth.authenticate_api_key("replacement-key").unwrap();
        assert_changed(
            tokio::time::timeout(WATCHDOG, processing)
                .await
                .unwrap()
                .unwrap(),
        );
        assert!(auth.is_authenticated());
        let _ = release.send(());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn ordinary_401_retains_its_status_after_ending_the_request_session() {
    let (url, _) = http_fixture(
        "401 Unauthorized",
        "application/json",
        r#"{"message":"expired"}"#,
    )
    .await;
    let (rest, auth) = rest_and_auth(url);
    assert!(
        matches!(rest.get("/account", None).await, Err(Error::Api(error)) if error.status == 401)
    );
    assert!(!auth.is_authenticated());
}

#[tokio::test]
async fn nested_guards_preserve_only_their_own_401_body_and_still_reject_replacement() {
    for replacement in [false, true] {
        let (url, _, release, server) = held_response(
            "401 Unauthorized",
            "application/json",
            "",
            "{\"message\":\"expired\"}",
            false,
        )
        .await;
        let (rest, auth) = rest_and_auth(url);
        let owner = auth.request_session();
        let (headers_received, headers) = oneshot::channel();
        let pending = tokio::spawn(async move {
            owner
                .run(async {
                    let nested = rest.request_session();
                    nested
                        .run(async {
                            let response = rest
                                .raw_request(
                                    reqwest::Method::GET,
                                    "/expired",
                                    None,
                                    None,
                                    None,
                                    None,
                                )
                                .await?;
                            let _ = headers_received.send(());
                            rest.process_response(response).await
                        })
                        .await
                })
                .await
        });
        headers.await.unwrap();
        assert!(!auth.is_authenticated());
        if replacement {
            auth.authenticate_api_key("B").unwrap();
            assert_changed(
                tokio::time::timeout(WATCHDOG, pending)
                    .await
                    .unwrap()
                    .unwrap(),
            );
            let _ = release.send(());
        } else {
            release.send(()).unwrap();
            assert!(
                matches!(tokio::time::timeout(WATCHDOG,pending).await.unwrap().unwrap(),Err(Error::Api(error)) if error.status == 401)
            );
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn another_requests_matching_401_does_not_keep_unrelated_work_alive() {
    let (url, _) = http_fixture("401 Unauthorized", "application/json", "{}").await;
    let (rest, auth) = rest_and_auth(url);
    let owner = auth.request_session();
    let (entered, started) = oneshot::channel();
    let pending = tokio::spawn(async move {
        owner
            .run(async {
                let _ = entered.send(());
                std::future::pending::<crate::Result<()>>().await
            })
            .await
    });
    started.await.unwrap();
    assert!(
        matches!(rest.get("/expired",None).await,Err(Error::Api(error)) if error.status == 401)
    );
    assert_changed(
        tokio::time::timeout(WATCHDOG, pending)
            .await
            .unwrap()
            .unwrap(),
    );
}

#[tokio::test]
async fn another_clients_matching_401_cannot_authorize_an_equal_numbered_epoch() {
    let (url, _, release, server) =
        held_response("401 Unauthorized", "application/json", "", "{}", false).await;
    let (rest, auth_b) = rest_and_auth(url.clone());
    let (_, auth_a) = rest_and_auth(url);
    let owner_a = auth_a.request_session();
    let owner_b = auth_b.request_session();
    assert_eq!(owner_a.id(), owner_b.id());
    let (entered, started) = oneshot::channel();
    let pending = tokio::spawn(async move {
        owner_a
            .run(async {
                owner_b
                    .run(async {
                        let response = rest
                            .raw_request(reqwest::Method::GET, "/expired", None, None, None, None)
                            .await?;
                        let _ = entered.send(());
                        rest.process_response(response).await
                    })
                    .await
            })
            .await
    });
    started.await.unwrap();
    auth_a.authenticate_api_key("replacement-A").unwrap();
    assert_eq!(auth_a.version().session, auth_b.version().session);
    assert_changed(
        tokio::time::timeout(WATCHDOG, pending)
            .await
            .unwrap()
            .unwrap(),
    );
    let _ = release.send(());
    server.await.unwrap();
}

#[tokio::test]
async fn sse_waiting_for_headers_or_idle_bytes_ends_with_its_account() {
    for hold_headers in [true, false] {
        let (url, started, release, server) = held_response(
            "200 OK",
            "text/event-stream",
            "",
            "data: old\n\n",
            hold_headers,
        )
        .await;
        let (rest, auth) = rest_and_auth(url);
        let operation = if hold_headers {
            tokio::spawn(async move {
                let mut stream = rest.stream_sse("/events", None, HeaderMap::new()).await?;
                stream.next().await.unwrap()
            })
        } else {
            let mut stream = rest
                .stream_sse("/events", None, HeaderMap::new())
                .await
                .unwrap();
            tokio::spawn(async move { stream.next().await.unwrap() })
        };
        started.await.unwrap();
        auth.clear();
        assert_changed(
            tokio::time::timeout(WATCHDOG, operation)
                .await
                .unwrap()
                .unwrap(),
        );
        let _ = release.send(());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn buffered_sse_events_are_checked_when_each_event_is_consumed() {
    let (url, _) = http_fixture(
        "200 OK",
        "text/event-stream",
        "data: first\n\ndata: old\n\n",
    )
    .await;
    let (rest, auth) = rest_and_auth(url);
    let mut stream = rest
        .stream_sse("/events", None, HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().data, "first");
    auth.authenticate_api_key("replacement-key").unwrap();
    assert_changed(stream.next().await.unwrap());
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn same_wallet_token_renewal_preserves_response_but_logout_and_relogin_do_not() {
    let (url, _, release, server) = held_response(
        "200 OK",
        "application/json",
        "",
        "{\"data\":\"retained\"}",
        false,
    )
    .await;
    let http = HttpClients::build(WATCHDOG).unwrap();
    let auth = AuthManager::new(
        AuthKind::Token,
        url.clone(),
        http.authenticated(),
        http.cookies(),
    );
    auth.authenticate_tokens(token("0xAbCd", 1, false), token("refresh", 1, false))
        .await
        .unwrap();
    let owner = auth.request_session();
    let rest = RestClient::new(url, auth.clone(), http, WATCHDOG);
    let response = rest
        .raw_request(reqwest::Method::GET, "/account", None, None, None, None)
        .await
        .unwrap();
    auth.authenticate_tokens(token("0xaBcD", 2, false), token("refresh", 2, false))
        .await
        .unwrap();
    owner.check().unwrap();
    release.send(()).unwrap();
    assert_eq!(
        rest.process_response(response).await.unwrap()["data"],
        "retained"
    );
    auth.clear();
    auth.authenticate_tokens(token("0xabcd", 3, false), token("refresh", 3, false))
        .await
        .unwrap();
    assert_changed(owner.check());
    server.await.unwrap();
}

#[tokio::test]
async fn new_account_token_renewal_is_not_blocked_by_the_old_refresh() {
    use axum::{Json, Router, routing::post};
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let request_started = started.clone();
    let response_release = release.clone();
    let router = Router::new().route("/v1/account/refresh-token", post(move |Json(body): Json<Value>| {
        let started = request_started.clone();
        let release = response_release.clone();
        async move {
            if body["refreshToken"] == token("A-refresh", 1, false) {
                started.notify_one();
                release.notified().await;
                Json(json!({"data":{"token":token("A", 2, false),"refreshToken":token("A-refresh", 2, false)}}))
            } else {
                Json(json!({"data":{"token":token("B", 2, false),"refreshToken":token("B-refresh", 2, false)}}))
            }
        }
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let http = HttpClients::build(Duration::from_secs(30)).unwrap();
    let auth = AuthManager::new(AuthKind::Token, url, http.authenticated(), http.cookies());
    let first = auth.clone();
    let pending = tokio::spawn(async move {
        first
            .authenticate_tokens(token("A", 1, true), token("A-refresh", 1, false))
            .await
    });
    tokio::time::timeout(WATCHDOG, started.notified())
        .await
        .unwrap();
    tokio::time::timeout(
        WATCHDOG,
        auth.authenticate_tokens(token("B", 1, true), token("B-refresh", 1, false)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_changed(
        tokio::time::timeout(WATCHDOG, pending)
            .await
            .unwrap()
            .unwrap(),
    );
    assert!(
        matches!(auth.backup().unwrap(), Some(crate::AuthBackup::Tokens { token: current, .. }) if current == token("B", 2, false))
    );
    release.notify_one();
    server.abort();
}
