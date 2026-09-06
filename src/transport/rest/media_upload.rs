//! Idempotent asset PUT retries never repeat a generation request.
use super::*;

const ATTEMPTS: u32 = 3;

pub(super) async fn put(
    rest: &RestClient,
    url: Url,
    data: Bytes,
    content_type: Option<&str>,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + rest.timeout;
    for attempt in 1..=ATTEMPTS {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(timeout());
        }
        let operation = put_once(rest, &url, &data, content_type, remaining);
        let result = tokio::time::timeout_at(deadline, operation)
            .await
            .map_err(|_| timeout())?;
        match result {
            Ok(()) => return Ok(()),
            Err(error) if attempt < ATTEMPTS && retryable(&error) => {
                let delay = retry_delay(&error, attempt);
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if delay >= remaining {
                    return Err(error);
                }
                tokio::time::sleep(delay).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(timeout())
}

async fn put_once(
    rest: &RestClient,
    url: &Url,
    data: &Bytes,
    content_type: Option<&str>,
    timeout: Duration,
) -> Result<()> {
    // Revalidate DNS on every connection. URL, payload and upload identity stay
    // unchanged; no authentication headers or cookie store reach the object host.
    let mut request = rest
        .media_client(url)
        .await?
        .put(url.clone())
        .body(data.clone())
        .timeout(timeout);
    if let Some(content_type) = content_type {
        request = request.header(reqwest::header::CONTENT_TYPE, content_type);
    }
    let response = request.send().await?;
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| crate::retry_after::seconds(value, chrono::Utc::now()));
    let mut error = ApiError::new(
        status.as_u16(),
        json!({
            "status": "error",
            "message": status.canonical_reason().unwrap_or("Failed to upload media"),
            "errorCode": 0,
            "retryAfterSeconds": retry_after,
        }),
    );
    error.retry_after_seconds = retry_after;
    Err(error.into())
}

fn retryable(error: &Error) -> bool {
    match error {
        Error::Api(error) => matches!(error.status, 408 | 425 | 429 | 500 | 502 | 503 | 504),
        Error::Http(error) => {
            error.is_timeout() || error.is_connect() || error.is_request() || error.is_body()
        }
        _ => false,
    }
}

fn retry_delay(error: &Error, attempt: u32) -> Duration {
    if let Error::Api(error) = error {
        if let Some(seconds) = error.retry_after_seconds.or_else(|| {
            error
                .payload
                .get("retryAfterSeconds")
                .and_then(Value::as_u64)
        }) {
            return Duration::from_secs(seconds);
        }
    }
    Duration::from_millis(u64::from(attempt) * 250)
}

fn timeout() -> Error {
    Error::Timeout("media upload deadline elapsed".into())
}

#[cfg(test)]
mod tests;
