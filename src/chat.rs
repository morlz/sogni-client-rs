//! Socket-native chat, hosted chat, durable chat-run, and hosted-tool APIs.

mod api;
mod auto_tools;
mod events;
mod hosted;
mod media;
mod model_routing;
mod runs;
mod socket;
mod tools;
mod types;
mod validation;

pub use api::ChatApi;
pub use model_routing::{
    BackboneModelOptions, BackboneModelSelectionReason, SelectedBackboneModel, VideoDefaults,
    filter_video_models_by_workflow, get_video_defaults, is_edit_image_model,
    resolve_hosted_tool_model_selector, select_backbone_model,
};
pub use tools::{HOSTED_TOOL_NAMES, HostedTools, is_sogni_tool_call, parse_tool_call_arguments};
pub use types::{
    ChatAutoToolCancellation, ChatAutoToolOptions, ChatChunk, ChatCompletion, ChatStream,
    ChatToolCall, ChatToolExecutionResult, ChatToolFunction, ChatToolHistoryEntry,
};
