use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::{
    types::WorkflowTemplatePage,
    validation::{require_id, require_object, required_template, template_data, valid_template},
};
use crate::{Result, transport::ApiClient, utils::path_segment};

/// CRUD and fork operations for saved workflow templates.
#[derive(Clone)]
pub struct CreativeWorkflowTemplatesApi {
    client: Arc<ApiClient>,
}

impl std::fmt::Debug for CreativeWorkflowTemplatesApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreativeWorkflowTemplatesApi")
            .finish_non_exhaustive()
    }
}

impl CreativeWorkflowTemplatesApi {
    pub(crate) fn new(client: Arc<ApiClient>) -> Self {
        Self { client }
    }

    pub async fn list(
        &self,
        visibility: Option<&str>,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<WorkflowTemplatePage> {
        let mut query = Map::new();
        if let Some(visibility) = visibility.filter(|value| !value.trim().is_empty()) {
            query.insert("visibility".into(), json!(visibility));
        }
        if let Some(offset) = offset {
            query.insert("offset".into(), json!(offset));
        }
        if let Some(limit) = limit {
            query.insert("limit".into(), json!(limit.clamp(1, 200)));
        }
        let query = Value::Object(query);
        let response = self
            .client
            .rest
            .get(
                "/v1/creative-agent/workflows/templates",
                query
                    .as_object()
                    .filter(|value| !value.is_empty())
                    .map(|_| &query),
            )
            .await?;
        let data = template_data(&response);
        let templates = data
            .get("templates")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| valid_template(item))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Ok(WorkflowTemplatePage {
            templates,
            next_cursor: data.get("next").and_then(Value::as_f64),
        })
    }

    pub async fn get(&self, template_id: &str) -> Result<Value> {
        require_id(template_id, "template_id")?;
        let response = self
            .client
            .rest
            .get(
                &format!(
                    "/v1/creative-agent/workflows/templates/{}",
                    path_segment(template_id)
                ),
                None,
            )
            .await?;
        required_template(&response, "")
    }

    pub async fn create(&self, template: &Value) -> Result<Value> {
        require_object(template, "template")?;
        let response = self
            .client
            .rest
            .post("/v1/creative-agent/workflows/templates", template)
            .await?;
        required_template(&response, "create")
    }

    pub async fn update(&self, template_id: &str, patch: &Value) -> Result<Value> {
        require_id(template_id, "template_id")?;
        require_object(patch, "patch")?;
        let response = self
            .client
            .rest
            .patch(
                &format!(
                    "/v1/creative-agent/workflows/templates/{}",
                    path_segment(template_id)
                ),
                patch,
            )
            .await?;
        required_template(&response, "update")
    }

    pub async fn delete(&self, template_id: &str) -> Result<()> {
        require_id(template_id, "template_id")?;
        self.client
            .rest
            .delete(&format!(
                "/v1/creative-agent/workflows/templates/{}",
                path_segment(template_id)
            ))
            .await?;
        Ok(())
    }

    pub async fn fork(&self, template_id: &str, new_name: Option<&str>) -> Result<Value> {
        require_id(template_id, "template_id")?;
        let body = new_name.map_or_else(|| json!({}), |name| json!({"newName": name}));
        let response = self
            .client
            .rest
            .post(
                &format!(
                    "/v1/creative-agent/workflows/templates/{}/fork",
                    path_segment(template_id)
                ),
                &body,
            )
            .await?;
        required_template(&response, "fork")
    }
}
