use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use super::{CurrentAccount, SubscriptionProjectionState, data};
use crate::{Error, Result, transport::ApiClient};

#[derive(Clone, Copy)]
pub(super) enum SubscriptionSource {
    Rest,
    Socket,
}

fn finite_number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
        .filter(|value| value.is_finite())
}

pub(super) fn subscription_version(value: &Value) -> Option<f64> {
    finite_number(
        value
            .get("subscription")
            .and_then(|value| value.get("version")),
    )
}

fn timestamp_millis_to_iso(value: &Value) -> Option<String> {
    let millis = finite_number(Some(value))?;
    if millis < i64::MIN as f64 || millis > i64::MAX as f64 {
        return None;
    }
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis as i64)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

pub(super) fn map_socket_subscription(data: &Value) -> Option<Value> {
    let data = data.as_object()?;
    let Some(subscription) = data.get("subscription").and_then(Value::as_object) else {
        return Some(json!({"active": false, "status": "none"}));
    };
    let Some(producer_status) = subscription.get("status").and_then(Value::as_str) else {
        return Some(json!({"active": false, "status": "none"}));
    };
    let active = data.get("active").and_then(Value::as_bool) == Some(true);
    let status = match producer_status {
        "grace" => "grace_period",
        "cancelled" if active => "cancel_at_period_end",
        "cancelled" | "revoked" => "canceled",
        "needs_reconciliation" => "past_due",
        "expired" => "expired",
        "trialing" => "trialing",
        "active" => "active",
        _ if active => "active",
        _ => "none",
    };
    let mut mapped = Map::new();
    mapped.insert("active".into(), json!(active));
    mapped.insert("status".into(), json!(status));
    for field in ["tier", "term", "provider", "scheduledTier", "scheduledTerm"] {
        if let Some(value) = subscription.get(field).filter(|value| match value {
            Value::Null => false,
            Value::Bool(value) => *value,
            Value::String(value) => !value.is_empty(),
            Value::Number(value) => value.as_f64() != Some(0.0),
            Value::Array(value) => !value.is_empty(),
            Value::Object(value) => !value.is_empty(),
        }) {
            mapped.insert(field.into(), value.clone());
        }
    }
    let period_end = if status == "grace_period" {
        subscription
            .get("graceEnd")
            .filter(|value| !value.is_null())
            .or_else(|| subscription.get("periodEnd"))
    } else {
        subscription.get("periodEnd")
    };
    for (source, target) in [
        (subscription.get("periodStart"), "currentPeriodStart"),
        (period_end, "currentPeriodEnd"),
        (subscription.get("scheduledChangeAt"), "scheduledChangeAt"),
    ] {
        if let Some(value) = source.and_then(timestamp_millis_to_iso) {
            mapped.insert(target.into(), json!(value));
        }
    }
    if let Some(value) = subscription
        .get("cancelAtPeriodEnd")
        .and_then(Value::as_bool)
        .or_else(|| (producer_status == "cancelled").then_some(true))
    {
        mapped.insert("cancelAtPeriodEnd".into(), json!(value));
    }
    if subscription.get("paymentPending").and_then(Value::as_bool) == Some(true) {
        mapped.insert("paymentPending".into(), json!(true));
    }
    let capabilities = subscription
        .get("capabilities")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| {
            if active
                && matches!(
                    subscription.get("tier").and_then(Value::as_str),
                    Some("unlimited" | "unlimited_pro")
                )
            {
                json!({"unlimited": true})
            } else {
                json!({})
            }
        });
    mapped.insert("capabilities".into(), capabilities);
    if let Some(fair_use) = subscription.get("fairUse").and_then(Value::as_object) {
        let reset_at = fair_use
            .get("resetAt")
            .and_then(|value| finite_number(Some(value)));
        if fair_use.get("limited").and_then(Value::as_bool) == Some(true)
            && reset_at.is_some_and(|value| value > chrono::Utc::now().timestamp_millis() as f64)
        {
            let mut normalized = fair_use.clone();
            if let Some(value) = fair_use.get("resetAt").and_then(timestamp_millis_to_iso) {
                normalized.insert("resetAt".into(), json!(value));
            }
            mapped.insert("fairUse".into(), Value::Object(normalized));
        }
    }
    Some(Value::Object(mapped))
}

pub(super) fn apply_free_spark_projection(current: &CurrentAccount, data: &Value) {
    let Some(locked) = data.get("freeSparkLocked").and_then(Value::as_bool) else {
        return;
    };
    let unlock_path = data
        .get("freeSparkUnlockPath")
        .and_then(Value::as_str)
        .filter(|path| matches!(*path, "trial" | "purchase"));
    current.update(json!({
        "freeSparkLocked": locked,
        "freeSparkUnlockPath": unlock_path,
    }));
}

pub(super) fn apply_subscription_projection(
    current: &CurrentAccount,
    state: &Mutex<SubscriptionProjectionState>,
    subscription: Value,
    source: SubscriptionSource,
    version: Option<f64>,
    socket_writes_at_start: Option<u64>,
) -> bool {
    let mut state = state.lock();
    if version
        .zip(state.last_version)
        .is_some_and(|(version, last)| version < last)
    {
        return false;
    }
    if matches!(source, SubscriptionSource::Rest)
        && version.is_none()
        && socket_writes_at_start.is_some_and(|writes| writes != state.socket_writes)
    {
        return false;
    }
    if let Some(version) = version {
        state.last_version = Some(version);
    }
    if matches!(source, SubscriptionSource::Socket) {
        state.socket_writes = state.socket_writes.saturating_add(1);
    }
    drop(state);
    current.update(json!({"subscription": subscription}));
    true
}

pub(super) async fn refresh_subscription_projection(
    client: &ApiClient,
    current: &CurrentAccount,
    state: &Mutex<SubscriptionProjectionState>,
) -> Result<Value> {
    let socket_writes = state.lock().socket_writes;
    let response = client.rest.get("/v1/subscriptions/status", None).await?;
    let subscription = data(&response)
        .get("subscription")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| {
            Error::Protocol("subscription response did not include data.subscription".into())
        })?;
    let version = finite_number(subscription.get("version"));
    let applied = apply_subscription_projection(
        current,
        state,
        subscription.clone(),
        SubscriptionSource::Rest,
        version,
        Some(socket_writes),
    );
    if applied {
        Ok(subscription)
    } else {
        Ok(current.subscription().unwrap_or(subscription))
    }
}

pub(super) fn event_network(value: &Value) -> Value {
    value
        .get("network")
        .cloned()
        .unwrap_or_else(|| value.clone())
}
