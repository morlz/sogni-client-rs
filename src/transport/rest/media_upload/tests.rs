use std::{collections::VecDeque, sync::Arc};

use axum::{Router, extract::State, http::HeaderMap, routing::put};
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;
use crate::transport::tests::api_key_rest;

#[derive(Clone)]
struct UploadState {
    statuses: Arc<Mutex<VecDeque<StatusCode>>>,
    bodies: Arc<Mutex<Vec<Bytes>>>,
    auth_leaked: Arc<Mutex<bool>>,
    retry_after: Option<&'static str>,
}

async fn upload(
    State(state): State<UploadState>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, HeaderMap, &'static str) {
    *state.auth_leaked.lock() |= ["api-key", "authorization", "cookie"]
        .iter()
        .any(|key| headers.contains_key(*key));
    state.bodies.lock().push(body);
    let status = state.statuses.lock().pop_front().unwrap_or(StatusCode::OK);
    let mut headers = HeaderMap::new();
    if let Some(retry_after) = state.retry_after {
        headers.insert(reqwest::header::RETRY_AFTER, retry_after.parse().unwrap());
    }
    (status, headers, "")
}

async fn fixture(
    statuses: &[StatusCode],
    retry_after: Option<&'static str>,
) -> (Url, UploadState, tokio::task::JoinHandle<()>) {
    let state = UploadState {
        statuses: Arc::new(Mutex::new(statuses.iter().copied().collect())),
        bodies: Default::default(),
        auth_leaked: Default::default(),
        retry_after,
    };
    let app = Router::new()
        .route("/", put(upload))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (
        Url::parse(&format!("http://{address}/")).unwrap(),
        state,
        server,
    )
}

#[tokio::test]
async fn transient_asset_puts_retry_the_same_bytes_without_credentials() {
    let (url, state, server) = fixture(
        &[
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::OK,
        ],
        None,
    )
    .await;
    let rest = api_key_rest(url.clone(), Duration::from_secs(3));
    let bytes = Bytes::from_static(b"immutable-guide");
    let result = rest.put_bytes(url, bytes.clone(), Some("image/png")).await;
    server.abort();
    assert!(result.is_ok());
    assert_eq!(*state.bodies.lock(), vec![bytes; 3]);
    assert!(!*state.auth_leaked.lock());
}

#[tokio::test]
async fn permanent_auth_failure_does_not_retry() {
    let (url, state, server) = fixture(&[StatusCode::FORBIDDEN, StatusCode::OK], None).await;
    let rest = api_key_rest(url.clone(), Duration::from_secs(3));
    let result = rest
        .put_bytes(url, Bytes::from_static(b"guide"), None)
        .await;
    server.abort();
    assert!(matches!(result, Err(Error::Api(error)) if error.status == 403));
    assert_eq!(state.bodies.lock().len(), 1);
}

#[tokio::test]
async fn interrupted_upload_retries_the_same_body_before_any_generation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.ends_with(b"immutable-guide") {
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            if index == 1 {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            }
        }
    });
    let url = Url::parse(&format!("http://{address}/")).unwrap();
    let rest = api_key_rest(url.clone(), Duration::from_secs(3));
    let result = rest
        .put_bytes(url, Bytes::from_static(b"immutable-guide"), None)
        .await;
    assert!(result.is_ok());
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn retry_count_and_retry_after_are_bounded_by_the_shared_deadline() {
    for (statuses, retry_after, expected) in [
        (vec![StatusCode::BAD_GATEWAY; 4], None, 3),
        (
            vec![StatusCode::TOO_MANY_REQUESTS, StatusCode::OK],
            Some("600"),
            1,
        ),
    ] {
        let (url, state, server) = fixture(&statuses, retry_after).await;
        let rest = api_key_rest(url.clone(), Duration::from_secs(2));
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            rest.put_bytes(url, Bytes::from_static(b"guide"), None),
        )
        .await;
        server.abort();
        assert!(result.unwrap().is_err());
        assert_eq!(state.bodies.lock().len(), expected);
    }
}
