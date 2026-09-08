use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Input hashes requested by the Sogni World pipeline.
///
/// Use with `appSource: "sogni-world"`. The service verifies uploaded inputs;
/// supplying a receipt does not establish their provenance on its own.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "stage",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorldGenerationReceiptRequest {
    TargetStill {
        source_image_sha256: String,
        selection_hash: String,
    },
    Transition {
        first_frame_sha256: String,
        last_frame_sha256: String,
    },
}

/// Worker-attested input/output hashes, when emitted by a generation workflow.
/// Ordinary projects and older workers may omit every field. Present hashes
/// are normalized to lowercase SHA-256 hex digests.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobProvenance {
    /// Hash of the exact completed artifact bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_image_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sam_prompt_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_rle_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_width: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sam_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_frame_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_frame_sha256: Option<String>,
}

pub(super) fn normalized_hash(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

impl JobProvenance {
    pub(super) fn from_result(data: &Value) -> Option<Self> {
        let mut result = Map::new();
        // Live results carry top-level fields; persisted records store them in
        // `result`. Only the public receipt fields enter the typed projection.
        for source in [data.get("result"), Some(data)].into_iter().flatten() {
            for field in [
                "sha256",
                "sourceImageSha256",
                "samPromptSha256",
                "maskRleSha256",
                "selectionHash",
                "firstFrameSha256",
                "lastFrameSha256",
            ] {
                if let Some(hash) = source.get(field).and_then(normalized_hash) {
                    result.insert(field.into(), json!(hash));
                }
            }
            for field in ["maskWidth", "maskHeight"] {
                if let Some(size) = source.get(field).and_then(Value::as_f64).filter(|size| {
                    size.is_finite()
                        && size.fract() == 0.0
                        && (1.0..=9_007_199_254_740_991.0).contains(size)
                }) {
                    result.insert(field.into(), json!(size as u64));
                }
            }
            if let Some(version) = source
                .get("samVersion")
                .and_then(Value::as_str)
                .filter(|v| {
                    !v.is_empty()
                        && v.len() <= 80
                        && v.as_bytes()[0].is_ascii_alphanumeric()
                        && v.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._:+-".contains(&b))
                })
            {
                result.insert("samVersion".into(), json!(version));
            }
        }
        if result.is_empty() {
            None
        } else {
            serde_json::from_value(Value::Object(result)).ok()
        }
    }
}
