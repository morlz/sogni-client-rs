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

mod media_upload;

pub type SseEvent = ParsedSseEvent;
pub type SseStream = Pin<Box<dyn Stream<Item = Result<SseEvent>> + Send>>;

#[derive(Clone)]
pub struct RestClient {
    base_url: Url,
    auth: AuthManager,
    authenticated_http: reqwest::Client,
    streaming_http: reqwest::Client,
    cookies: Arc<ClearableCookieStore>,
    media_http: reqwest::Client,
    timeout: Duration,
    strict_media: bool,
    media_proxy: Option<super::proxy::SocksProxy>,
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
            streaming_http: http.streaming,
            cookies: http.cookies,
            media_http: http.media,
            timeout,
            strict_media: http.strict_media,
            media_proxy: http.media_proxy,
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
        let url = self.url(path)?;
        let mut request = self
            .authenticated_http
            .request(method, url.clone())
            .timeout(timeout.unwrap_or(self.timeout));
        if let Some(query) = query {
            request = request.query(&query_pairs(query));
        }
        if let Some(body) = body {
            request = request.json(&drop_nulls(body.clone()));
        }
        self.send_authenticated(request, &url, headers).await
    }

    async fn send_authenticated(
        &self,
        request: reqwest::RequestBuilder,
        url: &Url,
        headers: Option<HeaderMap>,
    ) -> Result<reqwest::Response> {
        let (version, mut request_headers) = self.auth.headers().await?;
        if let Some(headers) = headers {
            request_headers.extend(headers);
        }
        let (cookie_generation, cookie) = self.cookies.request_header(url);
        if !request_headers.contains_key(COOKIE) {
            if let Some(mut cookie) = cookie {
                cookie.set_sensitive(true);
                request_headers.insert(COOKIE, cookie);
            }
        }
        if self.auth.version() != version {
            return Err(Error::InvalidInput(
                "account session changed before request".into(),
            ));
        }
        let response = request.headers(request_headers).send().await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            self.auth.clear_if_version(version);
        }
        self.cookies
            .store_response(cookie_generation, response.headers(), response.url());
        Ok(response)
    }

    pub async fn process_response(&self, response: reqwest::Response) -> Result<Value> {
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let text = response.text().await?;
        let parsed = if text.trim().is_empty() {
            None
        } else {
            serde_json::from_str::<Value>(&text).ok()
        };
        if !status.is_success() {
            let payload = parsed.filter(Value::is_object).unwrap_or_else(|| {
                let message = non_json_error_message(status, &text);
                json!({"status": "error", "message": message, "errorCode": status.as_u16()})
            });
            return Err(ApiError::new(status.as_u16(), payload)
                .with_retry_after(retry_after.as_deref())
                .into());
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
        media_upload::put(self, url, data, content_type).await
    }

    pub(crate) fn auth_updates(&self) -> tokio::sync::watch::Receiver<u64> {
        self.auth.subscribe_session()
    }

    /// Transfer a write-once saved asset using only the server's signed headers.
    pub(crate) async fn put_saved_asset(
        &self,
        url: Url,
        data: Bytes,
        headers: HeaderMap,
    ) -> Result<()> {
        let client = self.media_client_without_redirects(&url).await?;
        let response = client
            .put(url)
            .headers(headers)
            .body(data)
            .timeout(self.timeout.min(Duration::from_secs(300)))
            .send()
            .await?;
        let status = response.status();
        if status.is_success() || status == StatusCode::PRECONDITION_FAILED {
            Ok(())
        } else {
            Err(ApiError::new(
                status.as_u16(),
                json!({
                    "status": "error", "errorCode": 0,
                    "message": "Could not upload the selected file."
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
            .media_client(&url)
            .await?
            .post(url)
            .multipart(form.part("file", part))
            .timeout(self.timeout)
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
            .with_retry_after(response.headers().get(reqwest::header::RETRY_AFTER).and_then(|value| value.to_str().ok()))
            .into())
        }
    }

    pub async fn get_bytes(&self, url: Url) -> Result<Bytes> {
        let response = self
            .media_client(&url)
            .await?
            .get(url)
            .timeout(self.timeout)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.bytes().await?)
    }

    /// Fetch tool media without forwarding credentials, following redirects or
    /// buffering an unbounded response. Uses the client's configured proxy.
    pub(crate) async fn get_tool_media(&self, url: Url, max_bytes: usize) -> Result<Bytes> {
        let response = self
            .media_client_without_redirects(&url)
            .await?
            .get(url)
            .timeout(self.timeout)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(Error::Protocol(format!(
                "tool media download failed (HTTP {})",
                response.status().as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(Error::InvalidInput(
                "tool media exceeds the size limit".into(),
            ));
        }
        let mut stream = response.bytes_stream();
        let mut data = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if chunk.len() > max_bytes.saturating_sub(data.len()) {
                return Err(Error::InvalidInput(
                    "tool media exceeds the size limit".into(),
                ));
            }
            data.extend_from_slice(&chunk);
        }
        Ok(Bytes::from(data))
    }

    async fn media_client_without_redirects(&self, url: &Url) -> Result<reqwest::Client> {
        if self.strict_media {
            self.media_client(url).await
        } else {
            let mut builder =
                reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
            if let Some(proxy) = &self.media_proxy {
                builder = builder.proxy(proxy.http_proxy()?);
            }
            Ok(builder.build()?)
        }
    }

    async fn media_client(&self, url: &Url) -> Result<reqwest::Client> {
        if self.strict_media {
            super::media_policy::client(url, self.timeout, self.media_proxy.as_ref()).await
        } else {
            Ok(self.media_http.clone())
        }
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
        let url = self.url(path)?;
        let mut request = self.streaming_http.get(url.clone());
        if let Some(query) = query {
            request = request.query(&query_pairs(query));
        }
        let response = self
            .send_authenticated(request, &url, Some(headers))
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

fn non_json_error_message(status: StatusCode, text: &str) -> String {
    let body = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let label = status
        .canonical_reason()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
    if body.is_empty() {
        label
    } else if body.starts_with('<') {
        format!("{label}: {}", body.chars().take(200).collect::<String>())
    } else if body.chars().count() > 500 {
        format!("{}…", body.chars().take(500).collect::<String>())
    } else {
        body
    }
}

#[cfg(test)]
mod error_message_tests {
    use super::*;

    #[test]
    fn plain_text_explains_denial_and_html_retains_http_context() {
        assert_eq!(
            non_json_error_message(
                StatusCode::BAD_REQUEST,
                "  This model\n will be available soon. "
            ),
            "This model will be available soon."
        );
        assert_eq!(
            non_json_error_message(StatusCode::BAD_GATEWAY, "<html> bad gateway"),
            "Bad Gateway: <html> bad gateway"
        );
        assert_eq!(
            non_json_error_message(StatusCode::BAD_REQUEST, " \n "),
            "Bad Request"
        );
        assert_eq!(
            non_json_error_message(StatusCode::BAD_REQUEST, &"я".repeat(501))
                .chars()
                .count(),
            501
        );
    }
}
