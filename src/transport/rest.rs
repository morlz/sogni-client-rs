use std::{collections::BTreeMap, pin::Pin, sync::Arc, time::Duration};

use async_stream::try_stream;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::{
    Method, StatusCode,
    header::{COOKIE, HeaderMap},
};
use serde_json::{Value, json};
use url::Url;

use super::{ClearableCookieStore, HttpClients};
use crate::{
    ApiError, Error, Result,
    auth::AuthManager,
    utils::{ParsedSseEvent, drop_nulls, parse_sse_chunk, query_pairs},
};

pub type SseEvent = ParsedSseEvent;
pub type SseStream = Pin<Box<dyn Stream<Item = Result<SseEvent>> + Send>>;

#[derive(Clone)]
pub struct RestClient {
    base_url: Url,
    auth: AuthManager,
    authenticated_http: reqwest::Client,
    cookies: Arc<ClearableCookieStore>,
    media_http: reqwest::Client,
    timeout: Duration,
}

impl std::fmt::Debug for RestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestClient")
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl RestClient {
    pub(crate) fn new(
        base_url: Url,
        auth: AuthManager,
        http: HttpClients,
        timeout: Duration,
    ) -> Self {
        Self {
            base_url,
            auth,
            authenticated_http: http.authenticated,
            cookies: http.cookies,
            media_http: http.media,
            timeout,
        }
    }

    pub fn url(&self, path: &str) -> Result<Url> {
        let url = self.base_url.join(path.trim_start_matches('/'))?;
        if url.origin() != self.base_url.origin() {
            return Err(Error::InvalidInput(
                "authenticated REST paths must remain on the configured API origin".into(),
            ));
        }
        Ok(url)
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        query: Option<&Value>,
        body: Option<&Value>,
        headers: Option<HeaderMap>,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let response = self
            .raw_request(method, path, query, body, headers, timeout)
            .await?;
        self.process_response(response).await
    }

    pub async fn raw_request(
        &self,
        method: Method,
        path: &str,
        query: Option<&Value>,
        body: Option<&Value>,
        headers: Option<HeaderMap>,
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response> {
        let mut request_headers = self.auth.headers().await?;
        if let Some(headers) = headers {
            request_headers.extend(headers);
        }
        let url = self.url(path)?;
        let (cookie_generation, cookie) = self.cookies.request_header(&url);
        if !request_headers.contains_key(COOKIE) {
            if let Some(mut cookie) = cookie {
                cookie.set_sensitive(true);
                request_headers.insert(COOKIE, cookie);
            }
        }
        let mut request = self
            .authenticated_http
            .request(method, url)
            .headers(request_headers)
            .timeout(timeout.unwrap_or(self.timeout));
        if let Some(query) = query {
            request = request.query(&query_pairs(query));
        }
        if let Some(body) = body {
            request = request.json(&drop_nulls(body.clone()));
        }
        let response = request.send().await?;
        self.cookies
            .store_response(cookie_generation, response.headers(), response.url());
        Ok(response)
    }

    pub async fn process_response(&self, response: reqwest::Response) -> Result<Value> {
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED && self.auth.is_authenticated() {
            self.auth.clear();
        }
        let text = response.text().await?;
        let parsed = if text.trim().is_empty() {
            None
        } else {
            serde_json::from_str::<Value>(&text).ok()
        };
        if !status.is_success() {
            let payload = parsed.unwrap_or_else(|| {
                let excerpt = text
                    .chars()
                    .take(200)
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let base = status.canonical_reason().unwrap_or("HTTP request failed");
                let message = if excerpt.is_empty() {
                    base.to_owned()
                } else {
                    format!("{base}: {excerpt}")
                };
                json!({"status": "error", "message": message, "errorCode": status.as_u16()})
            });
            return Err(ApiError::new(status.as_u16(), payload).into());
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        parsed.ok_or_else(|| {
            Error::Protocol(format!(
                "failed to parse JSON response body (HTTP {})",
                status.as_u16()
            ))
        })
    }

    pub async fn get(&self, path: &str, query: Option<&Value>) -> Result<Value> {
        self.request(Method::GET, path, query, None, None, None)
            .await
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.request(Method::POST, path, None, Some(body), None, None)
            .await
    }

    pub async fn post_with(
        &self,
        path: &str,
        body: &Value,
        headers: HeaderMap,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        self.request(Method::POST, path, None, Some(body), Some(headers), timeout)
            .await
    }

    pub async fn patch(&self, path: &str, body: &Value) -> Result<Value> {
        self.request(Method::PATCH, path, None, Some(body), None, None)
            .await
    }

    pub async fn delete(&self, path: &str) -> Result<Value> {
        self.request(Method::DELETE, path, None, None, None, None)
            .await
    }

    pub async fn put_bytes(&self, url: Url, data: Bytes, content_type: Option<&str>) -> Result<()> {
        let mut request = self
            .media_http
            .put(url)
            .body(data)
            .timeout(Duration::from_secs(300));
        if let Some(content_type) = content_type {
            request = request.header(reqwest::header::CONTENT_TYPE, content_type);
        }
        let response = request.send().await?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            Err(ApiError::new(
                status.as_u16(),
                json!({
                    "status": "error",
                    "message": status.canonical_reason().unwrap_or("Failed to upload media"),
                    "errorCode": 0,
                }),
            )
            .into())
        }
    }

    pub async fn post_multipart(
        &self,
        url: Url,
        fields: &BTreeMap<String, String>,
        data: Bytes,
        file_name: &str,
        content_type: Option<&str>,
    ) -> Result<()> {
        let mut form = reqwest::multipart::Form::new();
        for (name, value) in fields {
            form = form.text(name.clone(), value.clone());
        }
        let mut part =
            reqwest::multipart::Part::bytes(data.to_vec()).file_name(file_name.to_owned());
        if let Some(content_type) = content_type {
            part = part.mime_str(content_type).map_err(|error| {
                Error::InvalidInput(format!("invalid media content type: {error}"))
            })?;
        }
        let response = self
            .media_http
            .post(url)
            .multipart(form.part("file", part))
            .timeout(Duration::from_secs(300))
            .send()
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            Err(ApiError::new(
                status.as_u16(),
                json!({"status": "error", "message": "Failed to upload media", "errorCode": status.as_u16()}),
            )
            .into())
        }
    }

    pub async fn get_bytes(&self, url: Url) -> Result<Bytes> {
        let response = self
            .media_http
            .get(url)
            .timeout(Duration::from_secs(300))
            .send()
            .await?
            .error_for_status()?;
        Ok(response.bytes().await?)
    }

    pub async fn stream_sse(
        &self,
        path: &str,
        query: Option<&Value>,
        mut headers: HeaderMap,
    ) -> Result<SseStream> {
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("text/event-stream"),
        );
        let response = self
            .raw_request(Method::GET, path, query, None, Some(headers), None)
            .await?;
        if !response.status().is_success() {
            self.process_response(response).await?;
            return Err(Error::Protocol("SSE endpoint returned no stream".into()));
        }
        let mut bytes = response.bytes_stream();
        let stream = try_stream! {
            let mut buffer = Vec::<u8>::new();
            let mut frame_lines = Vec::<String>::new();
            while let Some(chunk) = bytes.next().await {
                buffer.extend_from_slice(&chunk?);
                while let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
                    let line = buffer.drain(..=index).collect::<Vec<_>>();
                    let line = std::str::from_utf8(&line[..line.len().saturating_sub(1)])
                        .map_err(|error| Error::Protocol(format!("SSE stream is not UTF-8: {error}")))?
                        .trim_end_matches('\r')
                        .to_owned();
                    if line.is_empty() {
                        if !frame_lines.is_empty() {
                            let raw = frame_lines.join("\n");
                            for frame in parse_sse_chunk(&raw) {
                                yield frame;
                            }
                            frame_lines.clear();
                        }
                    } else {
                        frame_lines.push(line);
                    }
                }
            }
            if !buffer.is_empty() {
                frame_lines.push(String::from_utf8(buffer)
                    .map_err(|error| Error::Protocol(format!("SSE stream is not UTF-8: {error}")))?);
            }
            if !frame_lines.is_empty() {
                for frame in parse_sse_chunk(&frame_lines.join("\n")) {
                    yield frame;
                }
            }
        };
        Ok(Box::pin(stream))
    }
}
