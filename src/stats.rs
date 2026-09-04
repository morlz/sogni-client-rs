use std::sync::Arc;

use serde_json::Value;

use crate::{Result, transport::ApiClient};

/// Network leaderboard queries.
#[derive(Clone)]
pub struct StatsApi {
    client: Arc<ApiClient>,
}

impl std::fmt::Debug for StatsApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatsApi").finish_non_exhaustive()
    }
}

impl StatsApi {
    pub(crate) fn new(client: Arc<ApiClient>) -> Self {
        Self { client }
    }

    /// Return leaderboard data. Query keys are passed through to the public API.
    pub async fn leaderboard(&self, query: Option<&Value>) -> Result<Value> {
        let response = self.client.rest.get("/v1/leaderboard/", query).await?;
        Ok(response.get("data").cloned().unwrap_or(Value::Null))
    }
}
