use serde_json::{Value, json};

use super::{AccountApi, data, projection::refresh_subscription_projection};
use crate::{Error, Result};

impl AccountApi {
    pub async fn get_subscription_status(&self) -> Result<Value> {
        refresh_subscription_projection(&self.client, &self.current, &self.subscription_projection)
            .await
    }

    /// Refresh the current subscription projection from the authoritative REST endpoint.
    pub async fn refresh_subscription(&self) -> Result<Value> {
        self.get_subscription_status().await
    }

    pub async fn get_subscription_usage(&self) -> Result<Value> {
        let response = self
            .client
            .rest
            .get("/v1/subscriptions/usage", None)
            .await?;
        data(&response).get("usage").cloned().ok_or_else(|| {
            Error::Protocol("subscription response did not include data.usage".into())
        })
    }

    pub async fn get_trial_eligibility(&self) -> Result<Value> {
        let response = self
            .client
            .rest
            .get("/v1/subscriptions/trial-eligibility", None)
            .await?;
        Ok(data(&response).clone())
    }

    pub async fn set_device_id(&self, device_id: &str) -> Result<()> {
        self.client
            .rest
            .post("/v1/account/device-id", &json!({"deviceId": device_id}))
            .await?;
        Ok(())
    }

    pub async fn get_subscription_plans(&self) -> Result<Vec<Value>> {
        let response = self
            .client
            .rest
            .get("/v1/subscriptions/plans", None)
            .await?;
        data(&response)
            .get("plans")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                Error::Protocol("subscription response did not include data.plans".into())
            })
    }

    pub async fn create_subscription_checkout(&self, body: &Value) -> Result<Value> {
        let response = self
            .client
            .rest
            .post("/v1/iap/stripe/subscribe", body)
            .await?;
        Ok(data(&response).clone())
    }

    pub async fn create_subscription_portal_session(&self) -> Result<Value> {
        let response = self
            .client
            .rest
            .post("/v1/subscriptions/stripe/portal", &json!({}))
            .await?;
        Ok(data(&response).clone())
    }
}
