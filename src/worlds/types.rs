use std::{collections::BTreeMap, pin::Pin};

use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{Result, WorkloadAttribution};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildStatus {
    Queued,
    Running,
    WaitingForUser,
    Completed,
    PartialFailure,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildWaitingReason {
    CostApprovalRequired,
    ReviewRequired,
    InsufficientCredit,
    SafetyReviewRequired,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildWaiting {
    pub reason: WorldBuildWaitingReason,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<BTreeMap<String, Value>>,
    pub since: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildHotspotKind {
    Path,
    Moment,
    Collectible,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildAction {
    pub verb: String,
    pub subject: String,
    pub intent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
}

/// A normalized point in the scene image, used as a selection hint.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorldBuildPoint {
    pub x: f64,
    pub y: f64,
}

/// One path, moment or collectible to build. The service assigns missing task IDs.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildHotspotInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub kind: WorldBuildHotspotKind,
    pub object: String,
    pub action: WorldBuildAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leads_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point: Option<WorldBuildPoint>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildAudience {
    Everyone,
    Teen,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildTokenType {
    Spark,
    Sogni,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildBillingMode {
    Auto,
    Subscription,
    Tokens,
}

/// Start a build from a signed-in account. The server authorizes the request;
/// it requires cost confirmation before paid work and a verdict on each take.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartWorldBuildParams {
    pub world_id: String,
    pub node_id: String,
    pub hotspots: Vec<WorldBuildHotspotInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub look: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<WorldBuildAudience>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<WorldBuildTokenType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<WorldBuildBillingMode>,
    /// Reuse this key when retrying the same start; it is sent only as a header.
    #[serde(skip)]
    pub idempotency_key: Option<String>,
    #[serde(skip)]
    pub attribution: Option<WorkloadAttribution>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildTaskStep {
    Pending,
    Locating,
    Outlining,
    Reserving,
    RenderingStill,
    Directing,
    RenderingTransition,
    RenderingMoment,
    RenderingCollectible,
    Review,
    Approved,
    Rejected,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildPointLabel {
    Positive,
    Negative,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorldBuildSelectionPoint {
    pub x: f64,
    pub y: f64,
    pub label: WorldBuildPointLabel,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorldBuildBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildTaskSelection {
    pub id: String,
    pub selection_hash: String,
    pub project_id: String,
    pub points: Vec<WorldBuildSelectionPoint>,
    pub bounds: WorldBuildBounds,
    pub coverage: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct WorldBuildTaskProjects {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segmentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_still: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildReviewDecision {
    Approved,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorldBuildTaskReview {
    pub decision: WorldBuildReviewDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorldBuildTaskError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub at: String,
    pub step: WorldBuildTaskStep,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildTask {
    pub task_id: String,
    pub kind: WorldBuildHotspotKind,
    pub status: WorldBuildTaskStep,
    pub attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<WorldBuildTaskSelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction_id: Option<String>,
    pub projects: WorldBuildTaskProjects,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<WorldBuildTaskReview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WorldBuildTaskError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildCostLine {
    pub task_id: String,
    pub kind: WorldBuildHotspotKind,
    pub stages: BTreeMap<String, f64>,
    pub total: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildCostPreview {
    pub token_type: WorldBuildTokenType,
    pub billing_mode: WorldBuildBillingMode,
    pub lines: Vec<WorldBuildCostLine>,
    pub total: f64,
    pub covered_by_subscription: bool,
    pub issued_at: String,
    /// Confirmation must precede this timestamp; request a requote otherwise.
    pub valid_until: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorldBuildEvent {
    pub sequence: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<BTreeMap<String, Value>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildScope {
    pub owner_wallet_address: String,
    pub app_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<WorldBuildTokenType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<WorldBuildBillingMode>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildInput {
    pub world_id: String,
    pub node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub look: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<WorldBuildAudience>,
    pub hotspots: Vec<WorldBuildHotspotInput>,
    pub plan: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildAuthorization {
    pub accepted_total: f64,
    pub token_type: WorldBuildTokenType,
    pub preview_issued_at: String,
    pub accepted_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildTimestamps {
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// Persisted hosted build. Publish approved generations in the World studio.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorldBuildRecord {
    pub run_id: String,
    pub owner_wallet_address: String,
    pub world_id: String,
    pub node_id: String,
    pub status: WorldBuildStatus,
    pub scope: WorldBuildScope,
    pub input: WorldBuildInput,
    pub tasks: Vec<WorldBuildTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_preview: Option<WorldBuildCostPreview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization: Option<WorldBuildAuthorization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting: Option<WorldBuildWaiting>,
    /// List responses omit events; request their history or stream separately.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<WorldBuildEvent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancellation_reason: Option<String>,
    pub timestamps: WorldBuildTimestamps,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldBuildCostDecision {
    Confirm,
    Cancel,
    Requote,
}

/// An explicit response to the server's quote. Reading a quote never confirms it.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ConfirmWorldBuildCostParams {
    pub decision: WorldBuildCostDecision,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewWorldBuildTaskParams {
    pub task_id: String,
    pub decision: WorldBuildReviewDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListWorldBuildsOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Stop a local event subscription without cancelling the remote build.
#[derive(Clone, Debug, Default)]
pub struct WorldBuildEventCancellation {
    inner: CancellationToken,
}

impl WorldBuildEventCancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.inner.cancel();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    pub(super) async fn cancelled(&self) {
        self.inner.cancelled().await;
    }
}

#[derive(Clone, Debug, Default)]
pub struct StreamWorldBuildEventsOptions {
    /// Replay begins after this sequence, including when the cursor is zero.
    pub last_event_id: Option<u64>,
    pub cancellation: Option<WorldBuildEventCancellation>,
}

pub type WorldBuildEventStream = Pin<Box<dyn Stream<Item = Result<WorldBuildEvent>> + Send>>;
