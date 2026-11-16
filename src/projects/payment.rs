use super::*;

/// The server's current Premium Spark offer for a waiting plan project. The
/// quoted price and balance coverage are advisory until the owner accepts.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PayAsYouGoOffer {
    pub available: bool,
    pub cost_in_spark: String,
    pub covered_by_premium_spark: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PayAsYouGoRefusalReason {
    NotEligible,
    Started,
    NoIdleWorker,
    ConcurrencyLimit,
    InsufficientPremiumSpark,
    InsufficientFunds,
    PaymentFailed,
    StaffOnly,
    Unavailable,
    /// This request never went out. It says nothing about an earlier request.
    NotSent,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PayAsYouGoUnknownReason {
    Timeout,
    NotFound,
    SessionChanged,
    UnrecognizedReply,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PayAsYouGoSwitchOutcome {
    Switched {
        #[serde(rename = "paymentModel")]
        payment_model: String,
    },
    Refused {
        reason: PayAsYouGoRefusalReason,
    },
    /// The server may have already switched and charged the project.
    Unknown {
        reason: PayAsYouGoUnknownReason,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PayAsYouGoSwitchResult {
    pub project_id: String,
    #[serde(flatten)]
    pub outcome: PayAsYouGoSwitchOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_in_spark: Option<String>,
}

impl PayAsYouGoSwitchResult {
    pub(super) fn clears_offer(&self) -> bool {
        matches!(
            self.outcome,
            PayAsYouGoSwitchOutcome::Switched { .. }
                | PayAsYouGoSwitchOutcome::Refused {
                    reason: PayAsYouGoRefusalReason::Started | PayAsYouGoRefusalReason::NotEligible
                }
        )
    }

    pub(super) fn new(project_id: &str, outcome: PayAsYouGoSwitchOutcome) -> Self {
        Self {
            project_id: project_id.into(),
            outcome,
            message: None,
            cost_in_spark: None,
        }
    }
}

pub(super) fn normalize_offer(raw: &Value) -> Option<PayAsYouGoOffer> {
    if raw.get("available").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some(PayAsYouGoOffer {
        available: true,
        cost_in_spark: decimal(raw.get("costInSpark")?)?.to_owned(),
        covered_by_premium_spark: raw.get("coveredByPremiumSpark").and_then(Value::as_bool)
            == Some(true),
    })
}

pub(super) fn normalize_reply(project_id: &str, raw: &Value) -> PayAsYouGoSwitchResult {
    use PayAsYouGoSwitchOutcome::{Refused, Switched, Unknown};
    let reason = raw.get("reason").and_then(Value::as_str);
    let outcome = match raw.get("status").and_then(Value::as_str) {
        Some("switched") => Switched {
            payment_model: "paid_spark".into(),
        },
        Some("refused") if reason == Some("not_found") => Unknown {
            reason: PayAsYouGoUnknownReason::NotFound,
        },
        Some("refused") => {
            let reason = raw
                .get("reason")
                .and_then(|value| PayAsYouGoRefusalReason::deserialize(value).ok())
                .filter(|reason| *reason != PayAsYouGoRefusalReason::NotSent)
                .unwrap_or(PayAsYouGoRefusalReason::NotEligible);
            Refused { reason }
        }
        _ => Unknown {
            reason: PayAsYouGoUnknownReason::UnrecognizedReply,
        },
    };
    PayAsYouGoSwitchResult {
        project_id: project_id.into(),
        outcome,
        cost_in_spark: raw
            .get("costInSpark")
            .and_then(decimal)
            .map(ToOwned::to_owned),
        message: raw
            .get("message")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty() && value.encode_utf16().count() <= 600)
            .map(ToOwned::to_owned),
    }
}

fn decimal(value: &Value) -> Option<&str> {
    let s = value.as_str()?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let valid = s
        .split_once('.')
        .map_or_else(|| digits(s), |(a, b)| digits(a) && digits(b));
    (s.len() <= 64 && valid).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_decimal_precision_and_never_reports_not_found_as_refused() {
        let offer = normalize_offer(&json!({
            "available":true,"costInSpark":"0.000000000000000001","coveredByPremiumSpark":"true"
        }))
        .unwrap();
        assert_eq!(offer.cost_in_spark, "0.000000000000000001");
        assert!(!offer.covered_by_premium_spark);
        for invalid in [json!("1."), json!("-1"), json!("1e0"), json!(1)] {
            assert!(normalize_offer(&json!({"available":true,"costInSpark":invalid})).is_none());
        }
        let reply = normalize_reply("Example", &json!({"status":"refused","reason":"not_found"}));
        assert_eq!(
            json!(reply),
            json!({"status":"unknown","reason":"not_found","projectId":"Example"})
        );
        assert!(!reply.clears_offer());
    }
}
