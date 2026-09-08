#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![recursion_limit = "256"]

mod account;
mod announcements;
mod attribution;
mod auth;
mod chat;
mod client;
mod error;
mod event;
mod projects;
mod replay;
mod retry_after;
mod stats;
mod transport;
mod utils;
mod workflows;

pub use account::{AccountApi, CurrentAccount};
pub use announcements::AnnouncementsApi;
pub use attribution::{
    Attribution, ConnectionAttribution, ExecutionMode, InteractionKind, OperationScope,
    WorkloadAttribution, WorkloadKind,
};
pub use auth::{AuthBackup, AuthKind};
pub use chat::{
    ChatApi, ChatAutoToolCancellation, ChatAutoToolOptions, ChatChunk, ChatCompletion, ChatStream,
    ChatToolCall, ChatToolExecutionResult, ChatToolFunction, ChatToolHistoryEntry,
    HOSTED_TOOL_NAMES, HostedTools, is_sogni_tool_call, parse_tool_call_arguments,
};
pub use client::{ClientBuilder, ClientConfig, Network, SogniClient};
pub use error::{
    ApiError, ChatError, Error, ProjectError, Result, SUBSCRIPTION_ERROR_CODES,
    is_subscription_limit_error,
};
pub use event::{Event, EventReceiver};
pub use projects::{
    ACTIVE_PROJECTS_RECOVERED_EVENT, AssetRole, COMPLETED_PROJECTS_RECOVERED_EVENT, CostEstimate,
    Job, JobModelPhaseStep, JobPreparation, JobProvenance, JobSnapshot, JobStatus, MediaSource,
    ModelOptions, PROJECT_LOST_ORIGINAL_CODE, PresignedPost, Project, ProjectRequest,
    ProjectResolution, ProjectSnapshot, ProjectStatus, ProjectSubmissionError, ProjectsApi,
    ResolveMissingOptions, Sam3ImagePrompt, Sam3PointLabel, Sam3PromptBox, Sam3PromptPoint,
    SubmissionPhase, WorldGenerationReceiptRequest, is_project_lost_error, is_project_lost_payload,
};
pub use replay::{ReplayApi, ReplayGetResult, ReplayWriteResult};
pub use stats::StatsApi;
pub use transport::{ApiClient, RestClient, SseEvent, SseStream};
pub use utils::{
    LTX2_FRAME_STEP, MINIMAX_H3_BASE_FRAMES, MINIMAX_H3_DIMENSION_STEP, MINIMAX_H3_FPS,
    MINIMAX_H3_FRAME_STEP, MINIMAX_H3_MAX_DIMENSION, MINIMAX_H3_MAX_DURATION,
    MINIMAX_H3_MAX_FRAMES, MINIMAX_H3_MAX_PIXELS, MINIMAX_H3_MIN_DURATION, MINIMAX_H3_MIN_FRAMES,
    calculate_video_frames, detect_content_type, get_video_workflow_type, is_audio_model,
    is_external_video_model, is_happyhorse_model, is_ltx_model, is_minimax_h3_balanced_model,
    is_minimax_h3_model, is_minimax_h3_reference_model, is_minimax_h3_turbo_model,
    is_model_artifact_model, is_seedance_model, is_seedance25_model, is_video_model, is_wan_model,
    is_wan3_enhanced_model, is_wan3_model, new_id, parse_sse_chunk,
};
pub use workflows::{
    CREATIVE_WORKFLOW_WAITING_REASONS, CreativeWorkflowTemplatesApi, CreativeWorkflowsApi,
    ReseedWorkflowMetadata, ReseedWorkflowResult, ResumeWorkflowResult, WorkflowBillingOptions,
    WorkflowStart, WorkflowTemplatePage,
};

/// Version of the compatible upstream public client contract.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
