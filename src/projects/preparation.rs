use serde::{Deserialize, Serialize};

/// Whether a worker has begun or finished loading/unloading a model.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobModelPhaseStep {
    Start,
    End,
}

/// What the worker is doing while a job is initiating.
///
/// Model-switch phases each arrive at start and end; the end can include the
/// elapsed seconds. The model string is a worker class name, not a display name.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "phase",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum JobPreparation {
    DownloadingAssets {
        asset_type: String,
        requested: u32,
        cached: u32,
        total: u32,
        completed: u32,
        current: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_progress: Option<f64>,
    },
    UnloadingModel {
        model: String,
        step: JobModelPhaseStep,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_sec: Option<f64>,
    },
    LoadingModel {
        model: String,
        step: JobModelPhaseStep,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_sec: Option<f64>,
    },
}
