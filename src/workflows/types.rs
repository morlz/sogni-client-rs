use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::WorkloadAttribution;

/// Stable waiting reasons reported by durable workflows.
pub const CREATIVE_WORKFLOW_WAITING_REASONS: &[&str] = &[
    "ask_clarifying_question",
    "select_media_required",
    "cost_approval_required",
    "cost_reauthorization_required",
    "safety_review_required",
    "workflow_user_input_required",
    "insufficient_credit",
    "permission_required",
    "other",
];

pub(super) const TERMINAL_STATUSES: &[&str] =
    &["completed", "failed", "cancelled", "partial_failure"];

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowStart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_estimated_capacity_units: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm_cost: Option<bool>,
    /// Content-filter preference captured when this run starts. The service
    /// defaults it to true; resume and reseed preserve the original preference.
    #[serde(skip_serializing_if = "Option::is_none", alias = "safeContentFilter")]
    pub safe_content_filter: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_references: Option<Vec<Value>>,
    #[serde(skip)]
    pub idempotency_key: Option<String>,
    #[serde(skip)]
    pub attribution: Option<WorkloadAttribution>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowBillingOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_source: Option<String>,
    #[serde(skip)]
    pub attribution: Option<WorkloadAttribution>,
}

/// Result returned after resuming a paused durable workflow.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ResumeWorkflowResult {
    pub workflow: Value,
    pub resumed: bool,
}

/// Reseed metadata returned alongside the cloned workflow record.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ReseedWorkflowMetadata {
    pub cloned_from_run_id: String,
    /// Server step records, including every applied seed and future metadata.
    pub steps: Vec<Value>,
}

/// Result returned after cloning a durable workflow with fresh seeds.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ReseedWorkflowResult {
    pub workflow: Value,
    pub reseed: ReseedWorkflowMetadata,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorkflowTemplatePage {
    pub templates: Vec<Value>,
    pub next_cursor: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workflow_start_preserves_explicit_false_content_filter() {
        let start: WorkflowStart = serde_json::from_value(json!({
            "input": {}, "safeContentFilter": false
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(start).unwrap()["safe_content_filter"],
            false
        );
        assert!(
            serde_json::to_value(WorkflowStart::default())
                .unwrap()
                .get("safe_content_filter")
                .is_none()
        );
    }
}
