use crate::{Error, Result};

pub const LTX2_FRAME_STEP: i64 = 8;
pub const MINIMAX_H3_FPS: f64 = 24.0;
pub const MINIMAX_H3_FRAME_STEP: i64 = 17;
pub const MINIMAX_H3_BASE_FRAMES: i64 = 124;
pub const MINIMAX_H3_MIN_FRAMES: i64 = 124;
pub const MINIMAX_H3_MAX_FRAMES: i64 = 362;
pub const MINIMAX_H3_DIMENSION_STEP: i64 = 32;
pub const MINIMAX_H3_MAX_DIMENSION: i64 = 1_344;
pub const MINIMAX_H3_MAX_PIXELS: i64 = 1_032_192;
pub const MINIMAX_H3_MIN_DURATION: f64 = MINIMAX_H3_MIN_FRAMES as f64 / MINIMAX_H3_FPS;
pub const MINIMAX_H3_MAX_DURATION: f64 = MINIMAX_H3_MAX_FRAMES as f64 / MINIMAX_H3_FPS;

const WAN_MODELS: &[&str] = &[
    "wan_v2.2-14b-fp8_t2v",
    "wan_v2.2-14b-fp8_i2v",
    "wan_v2.2-14b-fp8_s2v",
    "wan_v2.2-14b-fp8_t2v_lightx2v",
    "wan_v2.2-14b-fp8_i2v_lightx2v",
    "wan_v2.2-14b-fp8_s2v_lightx2v",
    "wan_v2.2-14b-fp8_animate-move_lightx2v",
    "wan_v2.2-14b-fp8_animate-replace_lightx2v",
];
const SEEDANCE_MODELS: &[&str] = &[
    "seedance-2-0",
    "seedance-2-0-mini",
    "seedance-2-0-fast",
    "seedance-2-5",
];
const HAPPYHORSE_MODELS: &[&str] = &[
    "happyhorse-1.1-t2v",
    "happyhorse-1.1-i2v",
    "happyhorse-1.1-r2v",
];
const WAN3_MODELS: &[&str] = &["wan3.0-video", "wan3.0-spicy-video"];
const MINIMAX_H3_MODELS: &[&str] = &[
    "minimax-h3-fl2va-fp8_t2v",
    "minimax-h3-fl2va-fp8_i2v",
    "minimax-h3-fl2va-fp8_flf2v",
    "minimax-h3-ref2va-fp8_r2v",
    "minimax-h3-fl2va-fp8_t2v_turbo",
    "minimax-h3-fl2va-fp8_i2v_turbo",
    "minimax-h3-fl2va-fp8_flf2v_turbo",
    "minimax-h3-fastvideo-int8_t2v_turbo",
    "minimax-h3-fastvideo-int8_i2v_turbo",
    "minimax-h3-fastvideo-int8_flf2v_turbo",
    "minimax-h3-ref2va-fp8_r2v_turbo",
    "minimax-h3-fl2va-fp8_t2v_balanced",
    "minimax-h3-fl2va-fp8_i2v_balanced",
    "minimax-h3-fl2va-fp8_flf2v_balanced",
    "minimax-h3-ref2va-fp8_r2v_balanced",
];

#[must_use]
pub fn is_wan_model(model_id: &str) -> bool {
    WAN_MODELS.contains(&model_id)
}

#[must_use]
pub fn is_ltx_model(model_id: &str) -> bool {
    let workflow = ["t2v", "i2v", "a2v", "ia2v", "v2v"]
        .iter()
        .any(|workflow| model_id.contains(&format!("_{workflow}")));
    workflow
        && (model_id.starts_with("ltx2-19b-fp8_")
            || model_id.starts_with("ltx23-22b-fp8_")
            || model_id.starts_with("ltx25-22b-int8_"))
        || model_id == "ltx23-22b-10eros-v1.4-fp8mixed_i2v"
}

#[must_use]
pub fn is_seedance_model(model_id: &str) -> bool {
    SEEDANCE_MODELS.contains(&model_id)
}

#[must_use]
pub fn is_seedance25_model(model_id: &str) -> bool {
    model_id == "seedance-2-5"
}

#[must_use]
pub fn is_happyhorse_model(model_id: &str) -> bool {
    HAPPYHORSE_MODELS.contains(&model_id)
}

#[must_use]
pub fn is_wan3_model(model_id: &str) -> bool {
    WAN3_MODELS.contains(&model_id)
}

#[must_use]
pub fn is_wan3_enhanced_model(model_id: &str) -> bool {
    model_id == "wan3.0-spicy-video"
}

#[must_use]
pub fn is_minimax_h3_model(model_id: &str) -> bool {
    MINIMAX_H3_MODELS.contains(&model_id)
}

#[must_use]
pub fn is_minimax_h3_turbo_model(model_id: &str) -> bool {
    is_minimax_h3_model(model_id) && model_id.ends_with("_turbo")
}

#[must_use]
pub fn is_minimax_h3_balanced_model(model_id: &str) -> bool {
    is_minimax_h3_model(model_id) && model_id.ends_with("_balanced")
}

#[must_use]
pub fn is_minimax_h3_reference_model(model_id: &str) -> bool {
    is_minimax_h3_model(model_id) && get_video_workflow_type(model_id) == Some("r2v")
}

#[must_use]
pub fn is_external_video_model(model_id: &str) -> bool {
    is_seedance_model(model_id) || is_happyhorse_model(model_id) || is_wan3_model(model_id)
}

#[must_use]
pub fn is_video_model(model_id: &str) -> bool {
    is_wan_model(model_id)
        || is_ltx_model(model_id)
        || is_seedance_model(model_id)
        || is_happyhorse_model(model_id)
        || is_wan3_model(model_id)
        || is_minimax_h3_model(model_id)
}

#[must_use]
pub fn is_audio_model(model_id: &str) -> bool {
    model_id.starts_with("ace_step") || model_id == "minimax_music3"
}

#[must_use]
pub fn get_video_workflow_type(model_id: &str) -> Option<&'static str> {
    if is_wan3_model(model_id) {
        return Some("t2v");
    }
    if is_happyhorse_model(model_id) {
        return ["r2v", "i2v", "t2v"]
            .into_iter()
            .find(|kind| model_id.contains(&format!("-{kind}")));
    }
    if is_minimax_h3_model(model_id) {
        return ["r2v", "flf2v", "i2v", "t2v"]
            .into_iter()
            .find(|kind| model_id.contains(&format!("_{kind}")));
    }
    if !(is_wan_model(model_id) || is_ltx_model(model_id) || is_seedance_model(model_id)) {
        return None;
    }
    if model_id.contains("_i2v") {
        Some("i2v")
    } else if model_id.contains("_t2v") {
        Some("t2v")
    } else if (is_ltx_model(model_id) || is_seedance_model(model_id)) && model_id.contains("_v2v") {
        Some("v2v")
    } else if (is_ltx_model(model_id) || is_seedance_model(model_id)) && model_id.contains("_ia2v")
    {
        Some("ia2v")
    } else if is_ltx_model(model_id) && model_id.contains("_a2v") {
        Some("a2v")
    } else if is_wan_model(model_id) && model_id.contains("_s2v") {
        Some("s2v")
    } else if is_wan_model(model_id) && model_id.contains("_animate-move") {
        Some("animate-move")
    } else if is_wan_model(model_id) && model_id.contains("_animate-replace") {
        Some("animate-replace")
    } else {
        None
    }
}

pub fn calculate_video_frames(
    model_id: &str,
    duration: f64,
    fps: f64,
    min_frames: Option<i64>,
    max_frames: Option<i64>,
) -> Result<i64> {
    if !duration.is_finite() || duration < 0.0 || !fps.is_finite() || fps < 0.0 {
        return Err(Error::InvalidInput(
            "duration and fps must be finite non-negative numbers".into(),
        ));
    }
    let js_round = |value: f64| (value + 0.5).floor() as i64;
    let mut frames = if is_wan_model(model_id) {
        js_round(duration * 16.0) + 1
    } else if is_minimax_h3_model(model_id) {
        let requested = js_round(duration * MINIMAX_H3_FPS);
        let minimum = min_frames
            .unwrap_or(MINIMAX_H3_MIN_FRAMES)
            .max(MINIMAX_H3_MIN_FRAMES);
        let maximum = max_frames
            .unwrap_or(MINIMAX_H3_MAX_FRAMES)
            .min(MINIMAX_H3_MAX_FRAMES);
        let min_step = div_ceil(minimum - MINIMAX_H3_BASE_FRAMES, MINIMAX_H3_FRAME_STEP);
        let max_step = (maximum - MINIMAX_H3_BASE_FRAMES).div_euclid(MINIMAX_H3_FRAME_STEP);
        if min_step > max_step {
            return Err(Error::InvalidInput(format!(
                "no valid MiniMax H3 frame count exists between {minimum} and {maximum}"
            )));
        }
        let requested_step =
            js_round((requested - MINIMAX_H3_BASE_FRAMES) as f64 / MINIMAX_H3_FRAME_STEP as f64);
        return Ok(MINIMAX_H3_BASE_FRAMES
            + requested_step.clamp(min_step, max_step) * MINIMAX_H3_FRAME_STEP);
    } else {
        let raw = js_round(duration * fps) + 1;
        if is_ltx_model(model_id) {
            js_round((raw - 1) as f64 / LTX2_FRAME_STEP as f64) * LTX2_FRAME_STEP + 1
        } else {
            raw
        }
    };
    if let Some(minimum) = min_frames {
        frames = frames.max(minimum);
    }
    if let Some(maximum) = max_frames {
        frames = frames.min(maximum);
    }
    Ok(frames)
}

fn div_ceil(lhs: i64, rhs: i64) -> i64 {
    let quotient = lhs.div_euclid(rhs);
    if lhs.rem_euclid(rhs) == 0 {
        quotient
    } else {
        quotient + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_frames_match_the_reference_clients() {
        let cases = [
            ("wan_v2.2-14b-fp8_t2v", 5.0, 16.0, 81),
            ("wan_v2.2-14b-fp8_t2v", 5.0, 32.0, 81),
            ("ltx23-22b-fp8_t2v_dev", 5.0, 24.0, 121),
            ("ltx2-19b-fp8_t2v", 4.0, 24.0, 97),
            ("ltx23-22b-fp8_t2v_dev", 1.0, 25.0, 25),
            ("seedance-2-0-fast", 5.0, 24.0, 121),
            ("happyhorse-1.1-t2v", 3.0, 24.0, 73),
            ("seedance-2-0-fast", 0.125, 20.0, 4),
        ];
        for (model, duration, fps, expected) in cases {
            assert_eq!(
                calculate_video_frames(model, duration, fps, None, None).expect("frames"),
                expected,
                "{model}"
            );
        }
        assert_eq!(
            calculate_video_frames("ltx23-22b-fp8_t2v_dev", 1.0, 8.0, Some(17), None)
                .expect("minimum"),
            17
        );
        assert_eq!(
            calculate_video_frames("wan_v2.2-14b-fp8_t2v", 20.0, 32.0, None, Some(161))
                .expect("maximum"),
            161
        );
    }

    #[test]
    fn recognizes_quality_wan_sound_to_video() {
        let model = "wan_v2.2-14b-fp8_s2v";
        assert!(is_wan_model(model));
        assert!(is_video_model(model));
        assert_eq!(get_video_workflow_type(model), Some("s2v"));
        assert_eq!(
            calculate_video_frames(model, 5.0, 16.0, None, None).expect("WAN frames"),
            81
        );
    }
}
