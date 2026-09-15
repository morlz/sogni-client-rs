use super::*;

/// Standalone, promptless enhancement of a finished video.
pub const FLASHVSR_VIDEO_UPSCALE_MODEL_ID: &str = "flashvsr_v1.1_tiny_long_bf16";
pub const PIXAL3D_IMAGE_TO_3D_MODEL_ID: &str = "pixal3d_int8_i23d";
pub const PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID: &str = "pixal3d_multiview_int8_i23d";
pub const SAM3_IMAGE_SEGMENT_MODEL_ID: &str = "sam3_image_segment_bf16";
pub const BIREFNET_BACKGROUND_REMOVAL_MODEL_ID: &str = "birefnet_image_background_removal_fp16";
pub const MINIMAX_H3_FASTH3_IA2V_MODEL_ID: &str = "minimax-h3-fastvideo-int8_ia2v_turbo";
pub const MINIMAX_H3_FASTH3_FLFA2V_MODEL_ID: &str = "minimax-h3-fastvideo-int8_flfa2v_turbo";
pub const MINIMAX_H3_FASTH3_A2V_MODEL_ID: &str = "minimax-h3-fastvideo-int8_a2v_turbo";

/// Fixed upload slots for orbit views. Omitted views never renumber later views.
pub const PIXAL3D_ORBIT_VIEW_SLOTS: [(&str, u8); 3] = [
    ("leftViewImage", 1),
    ("backViewImage", 2),
    ("rightViewImage", 3),
];

#[must_use]
pub fn is_video_upscale_model(model_id: &str) -> bool {
    model_id == FLASHVSR_VIDEO_UPSCALE_MODEL_ID
}

#[must_use]
pub fn is_pixal3d_model(model_id: &str) -> bool {
    matches!(
        model_id,
        PIXAL3D_IMAGE_TO_3D_MODEL_ID | PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID
    )
}

#[must_use]
pub fn is_pixal3d_multi_view_model(model_id: &str) -> bool {
    model_id == PIXAL3D_MULTIVIEW_IMAGE_TO_3D_MODEL_ID
}

/// Segmentation returns a source-sized mask or RGBA cutout, not a generated image.
#[must_use]
pub fn is_segmentation_model(model_id: &str) -> bool {
    matches!(
        model_id,
        SAM3_IMAGE_SEGMENT_MODEL_ID | BIREFNET_BACKGROUND_REMOVAL_MODEL_ID
    )
}

#[must_use]
pub fn requires_starting_image(model_id: &str) -> bool {
    is_segmentation_model(model_id) || is_model_artifact_model(model_id)
}

#[must_use]
pub fn is_gpt_image_model(model_id: &str) -> bool {
    matches!(
        model_id,
        "gpt-image-2" | "gpt-image-2.5-sunburst" | "gpt-image-2.5-flare"
    )
}

/// Qwen3-TTS speech family. Use the runtime catalog for individual capabilities.
#[must_use]
pub fn is_speech_model(model_id: &str) -> bool {
    model_id.starts_with("qwen3_tts_")
}

/// FastH3 audio-guide output always carries the uploaded audio. Its window is
/// `frames / 24` seconds, optionally offset with `audioStart`.
#[must_use]
pub fn is_minimax_h3_audio_guide_model(model_id: &str) -> bool {
    let id = model_id.strip_suffix("_2stage").unwrap_or(model_id);
    matches!(
        id,
        MINIMAX_H3_FASTH3_IA2V_MODEL_ID
            | MINIMAX_H3_FASTH3_FLFA2V_MODEL_ID
            | MINIMAX_H3_FASTH3_A2V_MODEL_ID
    )
}

/// Smallest valid H3 frame count covering the audio, clamped to 124–362.
/// Exact grid durations retain their step; longer audio is trimmed by the service.
pub fn get_minimax_h3_frames_for_audio_duration(seconds: f64) -> Result<i64> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(Error::InvalidInput(
            "Audio duration must be a finite number of seconds greater than 0.".into(),
        ));
    }
    let needed = (seconds * MINIMAX_H3_FPS - 1e-6).ceil();
    let steps = ((needed - MINIMAX_H3_BASE_FRAMES as f64) / MINIMAX_H3_FRAME_STEP as f64)
        .ceil()
        .max(0.0);
    Ok(
        (MINIMAX_H3_BASE_FRAMES as f64 + steps * MINIMAX_H3_FRAME_STEP as f64)
            .min(MINIMAX_H3_MAX_FRAMES as f64) as i64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_frame_window_covers_duration_without_changing_exact_grid_values() {
        for (seconds, expected) in [
            (0.001, 124),
            (124.0 / 24.0, 124),
            (141.0 / 24.0, 141),
            (5.88, 158),
            (10.0, 243),
            (20.0, 362),
            (f64::MAX, 362),
        ] {
            assert_eq!(
                get_minimax_h3_frames_for_audio_duration(seconds).unwrap(),
                expected
            );
        }
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(get_minimax_h3_frames_for_audio_duration(invalid).is_err());
        }
    }

    #[test]
    fn video_model_recognition_includes_audio_guide_and_excludes_retired_720p_ids() {
        for mode in ["t2v", "i2v", "flf2v", "ia2v", "flfa2v", "a2v"] {
            for suffix in ["", "_2stage"] {
                let id = format!("minimax-h3-fastvideo-int8_{mode}_turbo{suffix}");
                assert!(is_minimax_h3_model(&id));
                assert!(is_minimax_h3_turbo_model(&id));
                assert!(is_video_model(&id));
                assert_eq!(get_video_workflow_type(&id), Some(mode));
                assert_eq!(
                    is_minimax_h3_audio_guide_model(&id),
                    ["ia2v", "flfa2v", "a2v"].contains(&mode)
                );
            }
        }
        for mode in ["t2v", "i2v", "flf2v"] {
            let retired = format!("minimax-h3-fastvideo-int8_{mode}_turbo_2stage_720p");
            assert!(!is_minimax_h3_model(&retired));
            assert!(!is_video_model(&retired));
            assert_eq!(get_video_workflow_type(&retired), None);
        }
        assert_eq!(
            calculate_video_frames(
                FLASHVSR_VIDEO_UPSCALE_MODEL_ID,
                158.0 / 24.0,
                24.0,
                None,
                None
            )
            .unwrap(),
            158
        );
    }
}
