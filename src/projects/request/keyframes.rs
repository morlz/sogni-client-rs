use super::*;

/// An intermediate still pinned at a 0-based, 24 fps MiniMax H3 frame.
///
/// The index must be unique and between 1 and the resolved video frame count
/// minus 2. Anchor images remain `ReferenceImage` and `ReferenceImageEnd`.
/// Name and describe each still as `<Picture N>` in the prompt, after the
/// workflow's own pictures. Use a new shot when its framing or light changes.
#[derive(Clone, Debug)]
pub struct MinimaxH3Keyframe {
    pub image: MediaSource,
    pub frame_index: u32,
}

impl MinimaxH3Keyframe {
    #[must_use]
    pub fn new(image: MediaSource, frame_index: u32) -> Self {
        Self { image, frame_index }
    }
}

impl ProjectRequest {
    /// Replace the intermediate MiniMax H3 keyframes, preserving caller order.
    ///
    /// At most eight images are accepted; validation runs before any uploads.
    /// Their `keyframeImage1..8` slots never displace reference-image slots.
    #[must_use]
    pub fn keyframes(mut self, keyframes: Vec<MinimaxH3Keyframe>) -> Self {
        self.assets
            .retain(|(role, _)| !matches!(role, AssetRole::KeyframeImage(_)));
        self.params.insert(
            "keyframes".into(),
            json!(
                keyframes
                    .iter()
                    .map(|entry| { json!({"image": true, "frameIndex": entry.frame_index}) })
                    .collect::<Vec<_>>()
            ),
        );
        // Oversized lists are retained for the serializer's count error, but
        // can never reach asset upload or overflow the numbered slot type.
        for (index, entry) in keyframes
            .into_iter()
            .take(MINIMAX_H3_MAX_KEYFRAMES)
            .enumerate()
        {
            self.assets
                .push((AssetRole::KeyframeImage((index + 1) as u8), entry.image));
        }
        self
    }
}
