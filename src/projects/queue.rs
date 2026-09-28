use super::*;

/// Current server explanation for queued work. Display `message` as plain text.
/// A free slot does not promise immediate processing; older servers omit this.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WaitingReason {
    pub reason: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_family: Option<String>,
}

/// One queued result, identified before a worker assigns its image ID.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobWaitingReason {
    pub job_index: u64,
    #[serde(rename = "imgID", default, skip_serializing_if = "Option::is_none")]
    pub img_id: Option<String>,
    pub waiting_reason: WaitingReason,
}

/// Complete current queue details emitted as the `queueChanged` API event.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectQueueChanged {
    pub project_id: String,
    pub waiting_reason: Option<WaitingReason>,
    pub job_waiting_reasons: Vec<JobWaitingReason>,
}

pub(super) fn normalize_waiting_reason(raw: &Value) -> Option<WaitingReason> {
    let value = raw.as_object()?;
    let reason = value.get("reason")?.as_str()?;
    if !matches!(
        reason,
        "concurrency_limit"
            | "model_concurrency_limit"
            | "payment_pending"
            | "no_workers"
            | "queued"
    ) {
        return None;
    }
    let message = value.get("message")?.as_str()?;
    if message.trim().is_empty() || message.encode_utf16().count() > 600 {
        return None;
    }
    let allowed = |field, values: &[&str]| {
        value
            .get(field)
            .and_then(Value::as_str)
            .filter(|text| values.contains(text))
            .map(str::to_owned)
    };
    Some(WaitingReason {
        reason: reason.into(),
        message: message.into(),
        media_type: allowed("mediaType", &["video", "media"]),
        payment_model: allowed(
            "paymentModel",
            &["subscription", "paid_spark", "free_spark", "sogni"],
        ),
        subscription_tier: allowed("subscriptionTier", &["unlimited", "unlimited_pro"]),
        model_family: allowed("modelFamily", &["minimax_h3"]),
    })
}

pub(super) fn normalize_job_waiting_reasons(raw: &Value, count: u32) -> Vec<JobWaitingReason> {
    let mut seen = HashSet::new();
    raw.as_array()
        .into_iter()
        .flatten()
        .take(count as usize)
        .filter_map(|row| {
            let index = row.get("jobIndex")?.as_u64()?;
            if index >= u64::from(count) || seen.contains(&index) {
                return None;
            }
            let waiting_reason = normalize_waiting_reason(row.get("waitingReason")?)?;
            seen.insert(index);
            Some(JobWaitingReason {
                job_index: index,
                img_id: row
                    .get("imgID")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.encode_utf16().count() <= 128)
                    .map(str::to_owned),
                waiting_reason,
            })
        })
        .collect()
}
