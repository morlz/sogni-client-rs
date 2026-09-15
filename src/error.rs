use std::fmt;

use serde_json::{Value, json};
use thiserror::Error;

/// Stable public subscription-related protocol codes.
pub const SUBSCRIPTION_ERROR_CODES: [(&str, i64); 4] = [
    ("NOT_ENTITLED", 4078),
    ("QUEUE_CAP", 4079),
    ("GRACE_RETRY", 4080),
    ("SUBSCRIPTION_FEATURE_REQUIRES_UPGRADE", 4081),
];

/// Public chat failures for which callers may submit a fresh request.
pub const RETRYABLE_CHAT_ERROR_TYPES: &[&str] = &["server_restarting", "transport_lost"];

#[derive(Clone, Debug)]
pub struct ApiError {
    pub status: u16,
    pub error_code: Value,
    pub message: String,
    pub payload: Value,
    /// Parsed server Retry-After advice. Reading it never retries a request.
    pub retry_after_seconds: Option<u64>,
}

impl ApiError {
    #[must_use]
    pub fn new(status: u16, payload: Value) -> Self {
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| format!("HTTP {status}"), ToOwned::to_owned);
        let error_code = payload
            .get("errorCode")
            .or_else(|| payload.get("error_code"))
            .cloned()
            .unwrap_or_else(|| json!(status));
        Self {
            status,
            error_code,
            message,
            payload,
            retry_after_seconds: None,
        }
    }

    #[must_use]
    pub fn with_retry_after(mut self, value: Option<&str>) -> Self {
        self.retry_after_seconds =
            value.and_then(|value| crate::retry_after::seconds(value, chrono::Utc::now()));
        self
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Clone, Debug)]
pub struct ProjectError {
    pub code: Option<Value>,
    pub message: String,
    pub payload: Value,
}

impl ProjectError {
    #[must_use]
    pub fn from_payload(payload: Value) -> Self {
        let code = payload.get("code").cloned();
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Project failed")
            .to_owned();
        Self {
            code,
            message,
            payload,
        }
    }
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProjectError {}

#[derive(Clone, Debug)]
pub struct ChatError {
    pub code: Option<String>,
    pub error_type: Option<String>,
    pub job_id: Option<String>,
    pub status: Option<u16>,
    pub message: String,
    pub payload: Value,
    pub subscription_limit: bool,
    pub required_plans: Vec<String>,
    pub feature: Option<String>,
    pub limitation: Option<String>,
}

impl ChatError {
    #[must_use]
    pub fn from_payload(payload: Value, status: Option<u16>, job_id: Option<String>) -> Self {
        let envelope = payload
            .get("error")
            .filter(|v| v.is_object())
            .unwrap_or(&payload);
        let subscription = envelope
            .get("subscription")
            .filter(|v| v.is_object())
            .unwrap_or(envelope);
        let code = envelope
            .get("code")
            .or_else(|| payload.get("error_code"))
            .map(value_string);
        let error_type = envelope
            .get("type")
            .or_else(|| payload.get("error"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let message = envelope
            .get("message")
            .or_else(|| payload.get("error_message"))
            .and_then(Value::as_str)
            .unwrap_or("Chat request failed")
            .to_owned();
        let required_plans = subscription
            .get("requiredPlans")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect();
        Self {
            code,
            error_type,
            job_id,
            status,
            message,
            subscription_limit: subscription
                .get("subscriptionLimit")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            required_plans,
            feature: subscription
                .get("feature")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            limitation: subscription
                .get("limitation")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            payload,
        }
    }

    #[must_use]
    pub fn subscription_error_code(&self) -> Option<i64> {
        let code = self.code.as_deref()?.parse::<i64>().ok()?;
        SUBSCRIPTION_ERROR_CODES
            .iter()
            .any(|(_, known)| *known == code)
            .then_some(code)
    }

    /// Whether this request was interrupted by the transport and may be sent again.
    #[must_use]
    pub fn retryable(&self) -> bool {
        self.error_type
            .as_deref()
            .is_some_and(|kind| RETRYABLE_CHAT_ERROR_TYPES.contains(&kind))
    }
}

#[must_use]
pub fn is_retryable_chat_error(error: &Error) -> bool {
    matches!(error, Error::Chat(error) if error.retryable())
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ChatError {}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    Chat(Box<ChatError>),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("operation timed out: {0}")]
    Timeout(String),
    #[error("client is closed")]
    Closed,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<ChatError> for Error {
    fn from(error: ChatError) -> Self {
        Self::Chat(Box::new(error))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[must_use]
pub fn is_subscription_limit_error(error: &Error) -> bool {
    match error {
        Error::Chat(error) => {
            error.subscription_limit || error.subscription_error_code() == Some(4081)
        }
        Error::Api(error) => {
            error
                .payload
                .get("subscriptionLimit")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || value_i64(&error.error_code) == Some(4081)
        }
        _ => false,
    }
}

pub(crate) fn value_string(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), ToOwned::to_owned)
}

pub(crate) fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
}
