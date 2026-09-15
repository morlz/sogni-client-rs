use serde::{Deserialize, Serialize};

/// A point's contribution to the selected foreground in a SAM3 mask.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sam3PointLabel {
    Positive,
    Negative,
}

/// Coordinates normalized to the original source image dimensions, from 0 to 1.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sam3PromptPoint {
    pub x: f64,
    pub y: f64,
    pub label: Sam3PointLabel,
}

/// Normalized top-left and bottom-right coordinates of a nonempty selection box.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sam3PromptBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    /// Positive by default; negative boxes exclude a text-prompted instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<Sam3PointLabel>,
}

/// Selection prompt for `sam3_image_segment_bf16`.
///
/// Supply points, boxes, or text. Text cannot be combined with points; points
/// support at most one positive box. Threshold defaults to 0.5. Point prompts
/// default to multimask; text/box prompts omit it. One source image produces
/// one lossless PNG mask or, with `apply_mask`, an RGBA source cutout.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sam3ImagePrompt {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<Sam3PromptPoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boxes: Vec<Sam3PromptBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multimask: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_mask: Option<bool>,
    /// Highest-scoring selections to retain, 1–16. Omitted keeps every match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_instances: Option<u8>,
}

/// Confidence and geometry of one concept instance or click candidate.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Sam3Selection {
    pub score: Option<f64>,
    /// Normalized `[x0, y0, x1, y1]`; absent for an empty selection.
    #[serde(rename = "box")]
    pub bounds: Option<[f64; 4]>,
    pub coverage: f64,
    pub included: bool,
}
