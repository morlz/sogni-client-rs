use std::fmt;

use serde::{Deserialize, Serialize};
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

/// The account must accept the model's agreement in a Sogni app before rendering.
/// The SDK does not accept agreements or automatically retry these refusals.
pub const MODEL_CONSENT_REQUIRED_ERROR_CODE: i64 = 4103;

/// The requested model is not yet available on the selected network.
pub const MODEL_NOT_YET_AVAILABLE_ERROR_CODE: i64 = 4104;

/// Public agreement reference returned with a model consent refusal.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelConsentRequirement {
    pub key: String,
    pub version: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ApiError {
    pub status: u16,
    pub error_code: Value,
    pub message: String,
    pub payload: Value,
    /// Server retry advice rounded up to whole seconds. The body's `retryAfter`
    /// takes precedence over the header. Reading it never retries a request.
    pub retry_after_seconds: Option<u64>,
}

impl ApiError {
    #[must_use]
    pub fn new(status: u16, payload: Value) -> Self {
        let fields = error_fields(&payload);
        let message = fields
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| format!("HTTP {status}"), ToOwned::to_owned);
        let error_code = fields
            .get("errorCode")
            .or_else(|| fields.get("error_code"))
            .cloned()
            .unwrap_or_else(|| json!(status));
        Self {
            status,
            error_code,
            message,
            retry_after_seconds: retry_after_seconds(&payload),
            payload,
        }
    }

    #[must_use]
    pub fn with_retry_after(mut self, value: Option<&str>) -> Self {
        self.retry_after_seconds = retry_after_seconds(&self.payload).or_else(|| {
            value.and_then(|value| crate::retry_after::seconds(value, chrono::Utc::now()))
        });
        self
    }

    /// The server's wait in seconds, preserving fractional body values.
    #[must_use]
    pub fn retry_after(&self) -> Option<f64> {
        retry_after(&self.payload).or(self.retry_after_seconds.map(|seconds| seconds as f64))
    }

    /// Structured context supplied by the service; arrays and scalars are ignored.
    #[must_use]
    pub fn details(&self) -> Option<&serde_json::Map<String, Value>> {
        error_fields(&self.payload)
            .get("details")
            .and_then(Value::as_object)
    }

    /// The agreement the service requires, without private diagnostic fields.
    #[must_use]
    pub fn consent_required(&self) -> Option<ModelConsentRequirement> {
        consent_requirement(error_fields(&self.payload))
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

    /// The agreement the service requires, without private diagnostic fields.
    #[must_use]
    pub fn consent_required(&self) -> Option<ModelConsentRequirement> {
        consent_requirement(&self.payload)
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
    /// Server retry advice rounded up to whole seconds, when available.
    pub retry_after_seconds: Option<u64>,
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
            retry_after_seconds: retry_after_seconds(&payload),
            payload,
        }
    }

    /// Retry advice for REST chat errors, with body values preferred over headers.
    #[must_use]
    pub fn retry_after(&self) -> Option<f64> {
        retry_after(&self.payload).or(self.retry_after_seconds.map(|seconds| seconds as f64))
    }

    #[must_use]
    pub fn details(&self) -> Option<&serde_json::Map<String, Value>> {
        error_fields(&self.payload)
            .get("details")
            .and_then(Value::as_object)
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

/// Recognize a model consent refusal from an SDK error. Reading it has no side effects.
#[must_use]
pub fn is_model_consent_required_error(error: &Error) -> bool {
    match error {
        Error::Api(error) => {
            is_model_consent_required_payload(&error.payload) || is_consent_code(&error.error_code)
        }
        Error::Project(error) => {
            is_model_consent_required_payload(&error.payload)
                || error.code.as_ref().is_some_and(is_consent_code)
        }
        Error::Chat(error) => {
            is_model_consent_required_payload(&error.payload)
                || error
                    .code
                    .as_ref()
                    .is_some_and(|code| is_consent_code(&json!(code)))
        }
        _ => false,
    }
}

/// Recognize numeric/string 4103 codes, `code`/`errorCode`, or a consent agreement.
#[must_use]
pub fn is_model_consent_required_payload(payload: &Value) -> bool {
    let fields = error_fields(payload);
    is_consent_code(fields)
        || fields
            .pointer("/consentRequired/key")
            .is_some_and(Value::is_string)
        || ["code", "errorCode"]
            .iter()
            .any(|key| fields.get(key).is_some_and(is_consent_code))
}

fn is_consent_code(value: &Value) -> bool {
    value.as_f64() == Some(MODEL_CONSENT_REQUIRED_ERROR_CODE as f64)
        || value.as_str().is_some_and(|code| {
            code.trim().parse::<f64>().ok() == Some(MODEL_CONSENT_REQUIRED_ERROR_CODE as f64)
        })
}

fn consent_requirement(payload: &Value) -> Option<ModelConsentRequirement> {
    serde_json::from_value(public_model_consent_requirement(
        payload.get("consentRequired")?,
    )?)
    .ok()
}

pub(crate) fn public_model_consent_requirement(value: &Value) -> Option<Value> {
    let key = value.get("key")?.as_str()?;
    let version = value.get("version").filter(|value| value.is_number())?;
    let mut requirement = json!({"key":key,"version":version});
    if let Some(model_id) = value.get("modelId").and_then(Value::as_str) {
        requirement["modelId"] = json!(model_id);
    }
    Some(requirement)
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

fn retry_after(payload: &Value) -> Option<f64> {
    error_fields(payload)
        .get("retryAfter")
        .and_then(Value::as_f64)
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
}

// Workflow, template and replay failures may put their public fields in data.
// Keep the original envelope intact in payload for forward compatibility.
fn error_fields(payload: &Value) -> &Value {
    if payload.get("status").and_then(Value::as_str) == Some("error") {
        payload
    } else {
        payload
            .get("data")
            .filter(|data| data.is_object())
            .unwrap_or(payload)
    }
}

fn retry_after_seconds(payload: &Value) -> Option<u64> {
    retry_after(payload).map(|seconds| seconds.ceil() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_consent_5_63_helpers_accept_public_codes_and_agreement_references() {
        for value in [
            json!(4103),
            json!(4103.0),
            json!("4103"),
            json!(" 4103 "),
            json!({"code":4103}),
            json!({"code":"4103"}),
            json!({"errorCode":"4103"}),
            json!({"code":0,"consentRequired":{"key":"agreement"}}),
        ] {
            assert!(is_model_consent_required_payload(&value), "{value}");
        }
        for value in [
            Value::Null,
            json!(false),
            json!(4104),
            json!("4104"),
            json!(""),
            json!({}),
            json!({"code":4081}),
            json!({"consentRequired":null}),
            json!({"consentRequired":{"key":false}}),
        ] {
            assert!(!is_model_consent_required_payload(&value), "{value}");
        }
        let payload = json!({"code":"4103","message":"Accept the agreement.","consentRequired":{"key":"agreement","version":2,"modelId":"seedance-2-0-mini-uncensored","privateDetail":"internal"}});
        let project = ProjectError::from_payload(payload.clone());
        let consent = project.consent_required().unwrap();
        assert_eq!(consent.key, "agreement");
        assert_eq!(consent.version, 2.0);
        assert_eq!(
            consent.model_id.as_deref(),
            Some("seedance-2-0-mini-uncensored")
        );
        assert!(is_model_consent_required_error(&Error::Project(project)));
        let api = ApiError::new(
            403,
            json!({"data":{"errorCode":"4103","consentRequired":payload["consentRequired"]}}),
        );
        assert_eq!(api.consent_required(), Some(consent));
        assert!(is_model_consent_required_error(&Error::Api(api)));
        assert!(!is_model_consent_required_error(&Error::Api(
            ApiError::new(403, json!({"errorCode":"4104"}))
        )));
        assert_eq!(MODEL_NOT_YET_AVAILABLE_ERROR_CODE, 4104);
    }

    #[test]
    fn rest_retry_advice_prefers_valid_body_and_preserves_payload() {
        for seconds in [0.0, 2.5, 1837.0] {
            let payload = json!({"message":"try later", "retryAfter":seconds,
                "details":{"reason":"busy"}});
            let error = ApiError::new(429, payload.clone()).with_retry_after(Some("9"));
            assert_eq!(error.retry_after(), Some(seconds));
            assert_eq!(error.retry_after_seconds, Some(seconds.ceil() as u64));
            assert_eq!(error.details().unwrap()["reason"], "busy");
            assert_eq!(error.payload, payload);
        }
        for value in [json!(-1), json!("60"), Value::Null, json!([])] {
            let error = ApiError::new(503, json!({"retryAfter":value,"details":[]}))
                .with_retry_after(Some("42"));
            assert_eq!(error.retry_after(), Some(42.0));
            assert!(error.details().is_none());
        }
        let plain = ApiError::new(400, json!({"message":"invalid"})).with_retry_after(Some("soon"));
        assert_eq!(plain.retry_after(), None);
        assert_eq!(plain.details(), None);
    }
}
