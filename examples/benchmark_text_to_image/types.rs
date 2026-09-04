use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct TierSteps {
    pub min: u32,
    pub max: u32,
    pub default: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub run: usize,
    pub is_warmup: bool,
    pub duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseResult {
    pub steps: u32,
    pub runs: Vec<RunRecord>,
    pub measured_count: usize,
    pub avg_ms: Option<f64>,
    pub median_ms: Option<f64>,
    pub min_ms: Option<u64>,
    pub max_ms: Option<u64>,
    pub warmup_ms: Option<u64>,
    pub failed_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostModel {
    pub base_ms: Option<i64>,
    pub per_step_ms: Option<i64>,
    pub default_derived_ms: Option<i64>,
    pub default_measured_ms: Option<i64>,
    pub default_error_ms: Option<i64>,
    pub default_error_pct: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResult {
    pub model_key: String,
    pub model_name: String,
    pub model_id: String,
    pub resolution: String,
    pub tier_steps: TierSteps,
    pub min_steps_benchmark: PhaseResult,
    pub default_steps_benchmark: PhaseResult,
    pub default_matches_min_or_max: bool,
    pub max_steps_benchmark: PhaseResult,
    pub cost_model: CostModel,
}
