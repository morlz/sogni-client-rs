use std::sync::Arc;

use serde_json::{Value, json};

use crate::{Error, Result, transport::ApiClient, utils::path_segment};

/// Read and dismiss account-scoped in-app announcements.
#[derive(Clone)]
pub struct AnnouncementsApi {
    client: Arc<ApiClient>,
}

impl std::fmt::Debug for AnnouncementsApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnnouncementsApi").finish_non_exhaustive()
    }
}

impl AnnouncementsApi {
    pub(crate) fn new(client: Arc<ApiClient>) -> Self {
        Self { client }
    }

    /// Return announcements currently visible to this account.
    pub async fn active(&self, platform: Option<&str>) -> Result<Vec<Value>> {
        let query = platform.map(|platform| json!({"platform": platform}));
        let response = self
            .client
            .rest
            .get("/v1/announcements/active", query.as_ref())
            .await?;
        Ok(active_announcements(&response))
    }

    /// Dismiss one announcement for this account on every device.
    pub async fn dismiss(&self, announcement_id: &str) -> Result<()> {
        if announcement_id.trim().is_empty() {
            return Err(Error::InvalidInput("announcement_id is required".into()));
        }
        self.client
            .rest
            .post(
                &format!(
                    "/v1/announcements/{}/dismiss",
                    path_segment(announcement_id)
                ),
                &json!({}),
            )
            .await?;
        Ok(())
    }
}

fn active_announcements(response: &Value) -> Vec<Value> {
    response
        .pointer("/data/announcements")
        .and_then(Value::as_array)
        .or_else(|| response.get("announcements").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_public_response_envelope() {
        let response = json!({
            "data": {"announcements": [{"id": "nested"}]},
            "announcements": [{"id": "legacy"}],
        });

        assert_eq!(
            active_announcements(&response),
            vec![json!({"id": "nested"})]
        );
    }

    #[test]
    fn preserves_empty_and_legacy_responses() {
        assert!(active_announcements(&json!({"data": {"announcements": []}})).is_empty());
        assert_eq!(
            active_announcements(&json!({"announcements": [{"id": "legacy"}]})),
            vec![json!({"id": "legacy"})]
        );
    }

    #[test]
    fn malformed_envelopes_are_safely_empty() {
        for response in [
            Value::Null,
            json!({}),
            json!({"data": null}),
            json!({"data": {"announcements": "not-an-array"}}),
            json!({"announcements": {"id": "not-an-array"}}),
        ] {
            assert!(active_announcements(&response).is_empty(), "{response}");
        }
    }
}
