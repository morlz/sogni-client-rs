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
    BIREFNET_BACKGROUND_REMOVAL_MODEL_ID, FLASHVSR_VIDEO_UPSCALE_MODEL_ID, LTX2_FRAME_STEP,
    MINIMAX_H3_BASE_FRAMES, MINIMAX_H3_DIMENSION_STEP, MINIMAX_H3_FASTH3_A2V_MODEL_ID,
    MINIMAX_H3_FASTH3_FLFA2V_MODEL_ID, MINIMAX_H3_FASTH3_IA2V_MODEL_ID, MINIMAX_H3_FPS,
    MINIMAX_H3_FRAME_STEP, MINIMAX_H3_MAX_DIMENSION, MINIMAX_H3_MAX_DURATION,
    MINIMAX_H3_MAX_FRAMES, MINIMAX_H3_MAX_PIXELS, MINIMAX_H3_MIN_DURATION, MINIMAX_H3_MIN_FRAMES,
    PIXAL3D_IMAGE_TO_3D_MODEL_ID, PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID, PIXAL3D_ORBIT_VIEW_SLOTS,
    SAM3_IMAGE_SEGMENT_MODEL_ID, calculate_video_frames, get_minimax_h3_frames_for_audio_duration,
    get_video_workflow_type, is_audio_model, is_external_video_model, is_gpt_image_model,
    is_happyhorse_model, is_ltx_model, is_minimax_h3_audio_guide_model,
    is_minimax_h3_balanced_model, is_minimax_h3_model, is_minimax_h3_reference_model,
    is_minimax_h3_turbo_model, is_model_artifact_model, is_pixal3d_model,
    is_pixal3d_multi_view_model, is_seedance_model, is_seedance25_model, is_segmentation_model,
    is_speech_model, is_video_model, is_video_upscale_model, is_wan_model, is_wan3_enhanced_model,
    is_wan3_model, requires_starting_image,
};
pub use sse::{ParsedSseEvent, parse_sse_chunk};
