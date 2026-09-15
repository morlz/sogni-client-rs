use super::*;
pub(in crate::projects) fn video_asset_requirements(
    model_id: &str,
    workflow: &str,
) -> BTreeMap<String, String> {
    let fields = if is_minimax_h3_reference_model(model_id) {
        [
            ("referenceImage", "optional"),
            ("referenceImageEnd", "forbidden"),
            ("referenceAudio", "optional"),
            ("referenceAudioIdentity", "forbidden"),
            ("referenceVideo", "optional"),
            ("referenceMask", "forbidden"),
        ]
    } else if is_minimax_h3_model(model_id) && workflow == "i2v" {
        [
            ("referenceImage", "optional"),
            ("referenceImageEnd", "optional"),
            ("referenceAudio", "forbidden"),
            ("referenceAudioIdentity", "forbidden"),
            ("referenceVideo", "forbidden"),
            ("referenceMask", "forbidden"),
        ]
    } else {
        match workflow {
            "upscale" => [
                ("referenceImage", "forbidden"),
                ("referenceImageEnd", "forbidden"),
                ("referenceAudio", "forbidden"),
                ("referenceAudioIdentity", "forbidden"),
                ("referenceVideo", "required"),
                ("referenceMask", "forbidden"),
            ],
            "t2v" => [
                ("referenceImage", "forbidden"),
                ("referenceImageEnd", "forbidden"),
                ("referenceAudio", "forbidden"),
                ("referenceAudioIdentity", "optional"),
                ("referenceVideo", "forbidden"),
                ("referenceMask", "forbidden"),
            ],
            "i2v" => [
                ("referenceImage", "optional"),
                ("referenceImageEnd", "optional"),
                ("referenceAudio", "forbidden"),
                ("referenceAudioIdentity", "optional"),
                ("referenceVideo", "forbidden"),
                ("referenceMask", "forbidden"),
            ],
            "flf2v" => [
                ("referenceImage", "required"),
                ("referenceImageEnd", "required"),
                ("referenceAudio", "forbidden"),
                ("referenceAudioIdentity", "forbidden"),
                ("referenceVideo", "forbidden"),
                ("referenceMask", "forbidden"),
            ],
            "flfa2v" => [
                ("referenceImage", "required"),
                ("referenceImageEnd", "required"),
                ("referenceAudio", "required"),
                ("referenceAudioIdentity", "forbidden"),
                ("referenceVideo", "forbidden"),
                ("referenceMask", "forbidden"),
            ],
            "s2v" | "ia2v" => [
                ("referenceImage", "required"),
                ("referenceImageEnd", "forbidden"),
                ("referenceAudio", "required"),
                ("referenceAudioIdentity", "forbidden"),
                ("referenceVideo", "forbidden"),
                ("referenceMask", "forbidden"),
            ],
            "a2v" => [
                ("referenceImage", "forbidden"),
                ("referenceImageEnd", "forbidden"),
                ("referenceAudio", "required"),
                ("referenceAudioIdentity", "forbidden"),
                ("referenceVideo", "forbidden"),
                ("referenceMask", "forbidden"),
            ],
            "animate-move" | "animate-replace" => [
                ("referenceImage", "required"),
                ("referenceImageEnd", "forbidden"),
                ("referenceAudio", "forbidden"),
                ("referenceAudioIdentity", "forbidden"),
                ("referenceVideo", "required"),
                ("referenceMask", "forbidden"),
            ],
            "v2v" => [
                ("referenceImage", "optional"),
                ("referenceImageEnd", "forbidden"),
                ("referenceAudio", "forbidden"),
                ("referenceAudioIdentity", "optional"),
                ("referenceVideo", "required"),
                ("referenceMask", "optional"),
            ],
            "r2v" => [
                ("referenceImage", "optional"),
                ("referenceImageEnd", "forbidden"),
                ("referenceAudio", "forbidden"),
                ("referenceAudioIdentity", "forbidden"),
                ("referenceVideo", "forbidden"),
                ("referenceMask", "forbidden"),
            ],
            _ => return BTreeMap::new(),
        }
    };
    fields
        .into_iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect()
}

pub(in crate::projects) fn custom_image_size_bounds(model_id: &str) -> (f64, f64) {
    if model_id == "rtx_vsr_pro" {
        (512.0, 15_360.0)
    } else if matches!(
        model_id,
        "krea2_identity_edit_v1_2" | "dark_beast_krea2_identity_edit_v1_2"
    ) {
        (512.0, 2_048.0)
    } else if is_gpt_image_model(model_id) {
        (256.0, 3_840.0)
    } else if matches!(
        model_id,
        "z_image_bf16"
            | "z_image_turbo_bf16"
            | "krea2_turbo_fp8_scaled"
            | "qwen_image_edit_2511_fp8"
            | "qwen_image_edit_2511_fp8_lightning"
            | "qwen_image_2512_fp8"
            | "qwen_image_2512_fp8_lightning"
    ) {
        (256.0, 2_560.0)
    } else {
        (256.0, 2_048.0)
    }
}
