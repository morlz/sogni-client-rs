//! Protocol-compatible model, media, JSON, identifier, and SSE helpers.

mod codec;
mod json;
mod media;
mod models;
mod sse;

pub use codec::new_id;
pub(crate) use codec::{b64_json_decode, b64_json_encode, path_segment};
pub use json::drop_nulls;
pub(crate) use json::{query_pairs, scalar_string};
pub use media::detect_content_type;
pub use models::{
    LTX2_FRAME_STEP, MINIMAX_H3_BASE_FRAMES, MINIMAX_H3_DIMENSION_STEP, MINIMAX_H3_FPS,
    MINIMAX_H3_FRAME_STEP, MINIMAX_H3_MAX_DIMENSION, MINIMAX_H3_MAX_DURATION,
    MINIMAX_H3_MAX_FRAMES, MINIMAX_H3_MAX_PIXELS, MINIMAX_H3_MIN_DURATION, MINIMAX_H3_MIN_FRAMES,
    calculate_video_frames, get_video_workflow_type, is_audio_model, is_external_video_model,
    is_happyhorse_model, is_ltx_model, is_minimax_h3_balanced_model, is_minimax_h3_model,
    is_minimax_h3_reference_model, is_minimax_h3_turbo_model, is_seedance_model,
    is_seedance25_model, is_video_model, is_wan_model, is_wan3_enhanced_model, is_wan3_model,
};
pub use sse::{ParsedSseEvent, parse_sse_chunk};
