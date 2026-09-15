use super::Sam3Selection;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Input hashes requested by the Sogni World pipeline.
///
/// The service verifies uploaded inputs. A receipt binds inputs without
/// selecting a generation recipe or establishing provenance on its own.
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
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
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
    pub mask_box: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_coverage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_detected_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_returned_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask_selections: Option<Vec<Sam3Selection>>,
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
            if let Some(bounds) = source.get("maskBox").and_then(normalized_bounds) {
                result.insert("maskBox".into(), json!(bounds));
            }
            if let Some(coverage) = source.get("maskCoverage").and_then(unit_interval) {
                result.insert("maskCoverage".into(), json!(coverage));
            }
            for field in ["maskDetectedCount", "maskReturnedCount"] {
                if let Some(count) = source.get(field).and_then(Value::as_f64).filter(|count| {
                    count.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(count)
                }) {
                    result.insert(field.into(), json!(count as u64));
                }
            }
            if let Some(selections) = source.get("maskSelections").and_then(Value::as_array) {
                let valid: Vec<_> = selections
                    .iter()
                    .filter_map(|selection| {
                        let score = selection.get("score")?;
                        let score = if score.is_null() {
                            None
                        } else {
                            Some(unit_interval(score)?)
                        };
                        Some(Sam3Selection {
                            score,
                            bounds: selection.get("box").and_then(normalized_bounds),
                            coverage: selection.get("coverage").and_then(unit_interval)?,
                            included: selection.get("included")?.as_bool()?,
                        })
                    })
                    .collect();
                if !valid.is_empty() {
                    result.insert("maskSelections".into(), json!(valid));
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

fn unit_interval(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| (0.0..=1.0).contains(v))
}

fn normalized_bounds(value: &Value) -> Option<[f64; 4]> {
    let values = value.as_array()?;
    if values.len() != 4 {
        return None;
    }
    let bounds = [
        unit_interval(&values[0])?,
        unit_interval(&values[1])?,
        unit_interval(&values[2])?,
        unit_interval(&values[3])?,
    ];
    (bounds[0] < bounds[2] && bounds[1] < bounds[3]).then_some(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_geometry_keeps_only_valid_public_fields() {
        let data = json!({"maskBox":[0.1,0.2,0.9,1], "maskCoverage":0.3,
            "maskDetectedCount":5, "maskReturnedCount":1, "maskSelections":[
                {"score":0.9,"box":[0.1,0.2,0.9,1],"coverage":0.3,"included":true},
                {"score":null,"box":[1,1,0,0],"coverage":0,"included":false},
                {"score":-1,"coverage":0.5,"included":false},
                {"coverage":0.5,"included":true}]});
        let receipt = JobProvenance::from_result(&data).unwrap();
        assert_eq!(receipt.mask_detected_count, Some(5));
        let selections = receipt.mask_selections.unwrap();
        assert_eq!(selections.len(), 2);
        assert!(selections[1].bounds.is_none());
        assert!(
            JobProvenance::from_result(&json!({"maskCoverage":2,"maskBox":[1,1,0,0]})).is_none()
        );
    }
}
