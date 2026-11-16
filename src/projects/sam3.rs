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

/// Track one selected object in the original video. Supply text, points, or
/// boxes on one source frame. The result is a hard mask at the source timing.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sam3VideoPrompt {
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
    /// Zero-based source frame for this selection (default 0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_index: Option<u32>,
    /// Zero-based candidate in confidence order (default 0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_index: Option<u8>,
    /// Draw a region on the selected frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<Sam3PromptMask>,
    /// Track multiple selections together and return their union.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objects: Option<Vec<Sam3VideoSelection>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combine: Option<Sam3Combine>,
    /// Selections on different source frames for the same object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<Sam3VideoFramePrompt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Sam3Memory>,
}

/// A drawn region on the input frame. Coordinates are normalized, and
/// nested rings represent holes. The service validates it against the source.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Sam3PromptMask {
    pub polygons: Vec<Vec<[f64; 2]>>,
}

/// One selection in a multiple-object or multiple-frame SAM 3 video request.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sam3VideoSelection {
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
    pub mask: Option<Sam3PromptMask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_index: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sam3VideoFramePrompt {
    pub frame_index: u32,
    #[serde(flatten)]
    pub selection: Sam3VideoSelection,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sam3Combine {
    Union,
}

/// How the tracker carries a selection between chunks.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sam3Memory {
    Chunk,
    Anchor,
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
