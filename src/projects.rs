//! Image, video, audio, segmentation, and 3D artifact generation projects.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use bytes::Bytes;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, Notify};
use url::Url;

use crate::{
    Error, EventReceiver, Network, ProjectError, Result, WorkloadAttribution,
    event::EventBus,
    transport::ApiClient,
    utils::{
        MINIMAX_H3_BASE_FRAMES, MINIMAX_H3_DIMENSION_STEP, MINIMAX_H3_FRAME_STEP,
        MINIMAX_H3_MAX_DIMENSION, MINIMAX_H3_MAX_DURATION, MINIMAX_H3_MAX_FRAMES,
        MINIMAX_H3_MAX_PIXELS, MINIMAX_H3_MIN_DURATION, MINIMAX_H3_MIN_FRAMES,
        calculate_video_frames, detect_content_type, get_video_workflow_type, is_audio_model,
        is_external_video_model, is_happyhorse_model, is_ltx_model, is_minimax_h3_balanced_model,
        is_minimax_h3_model, is_minimax_h3_reference_model, is_minimax_h3_turbo_model,
        is_model_artifact_model, is_seedance_model, is_seedance25_model, is_video_model,
        is_wan3_enhanced_model, is_wan3_model, new_id, path_segment, scalar_string,
    },
};

mod api;
mod events;
mod helpers;
mod job;
mod mapping;
mod preparation;
mod project;
mod provenance;
mod recovery;
mod request;
mod sam3;
mod snapshots;
mod submission;
mod validation;
mod wire;

#[cfg(test)]
mod tests;

pub use api::ProjectsApi;
pub use job::Job;
pub use preparation::{JobModelPhaseStep, JobPreparation};
pub use project::Project;
pub use provenance::{JobProvenance, WorldGenerationReceiptRequest};
pub use recovery::{
    ACTIVE_PROJECTS_RECOVERED_EVENT, COMPLETED_PROJECTS_RECOVERED_EVENT,
    PROJECT_LOST_ORIGINAL_CODE, ProjectResolution, ResolveMissingOptions, is_project_lost_error,
    is_project_lost_payload,
};
pub use request::{AssetRole, MediaSource, ProjectRequest};
pub use sam3::{Sam3ImagePrompt, Sam3PointLabel, Sam3PromptBox, Sam3PromptPoint};
pub use snapshots::{
    CostEstimate, JobSnapshot, JobStatus, ModelOptions, PresignedPost, ProjectSnapshot,
    ProjectStatus,
};
pub use submission::{ProjectSubmissionError, SubmissionPhase};

use events::{cancel_project, listen_for_project_events};
use helpers::*;
use mapping::*;
use recovery::{
    is_llm_recovery, project_lost_payload, raw_result_url, recovered_params, replay_recovered,
};
use request::{effective_asset_wire_name, mark_asset_param, validate_asset_roles};
use validation::{
    custom_image_size_bounds, validate_h3_params, validate_project_params, validate_video_assets,
    video_asset_requirements,
};
use wire::build_job_request;

const MODEL_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(120);
const RECENTLY_CREATED_GRACE: Duration = Duration::from_secs(5);
const MISSING_PROJECT_ATTEMPTS: usize = 4;
const MISSING_PROJECT_RETRY: Duration = Duration::from_millis(2_500);
const MINIMAX_H3_MAX_REFERENCE_IMAGES: usize = 9;
const MINIMAX_H3_MAX_REFERENCE_VIDEOS: usize = 3;
const MINIMAX_H3_MAX_REFERENCE_AUDIOS: usize = 3;
const MINIMAX_H3_MAX_REFERENCE_FILES: usize = 12;

fn enhancement_strength(strength: &str) -> f64 {
    match strength {
        "light" => 0.15,
        "heavy" => 0.49,
        _ => 0.35,
    }
}
