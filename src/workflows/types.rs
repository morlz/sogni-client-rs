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
