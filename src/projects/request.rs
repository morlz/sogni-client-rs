use super::*;

mod builder;
pub(in crate::projects) use builder::{
    effective_asset_wire_name, mark_asset_param, validate_asset_roles,
};

/// A local file or in-memory media payload to upload before a project starts.
#[derive(Clone, Debug)]
pub enum MediaSource {
    Path(PathBuf),
    Bytes {
        data: Bytes,
        file_name: Option<String>,
        content_type: Option<String>,
    },
}

impl MediaSource {
    #[must_use]
    pub fn bytes(data: impl Into<Bytes>) -> Self {
        Self::Bytes {
            data: data.into(),
            file_name: None,
            content_type: None,
        }
    }

    #[must_use]
    pub fn named_bytes(
        data: impl Into<Bytes>,
        file_name: impl Into<String>,
        content_type: impl Into<String>,
    ) -> Self {
        Self::Bytes {
            data: data.into(),
            file_name: Some(file_name.into()),
            content_type: Some(content_type.into()),
        }
    }

    pub(super) async fn read(&self) -> Result<LoadedMedia> {
        match self {
            Self::Path(path) => {
                let data = Bytes::from(tokio::fs::read(path).await?);
                let content_type = detect_content_type(Some(path), &data);
                let file_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("upload.bin")
                    .to_owned();
                Ok(LoadedMedia {
                    data,
                    file_name,
                    content_type,
                })
            }
            Self::Bytes {
                data,
                file_name,
                content_type,
            } => Ok(LoadedMedia {
                data: data.clone(),
                file_name: file_name.clone().unwrap_or_else(|| "upload.bin".into()),
                content_type: content_type
                    .clone()
                    .or_else(|| detect_content_type(None, data)),
            }),
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum AssetRole {
    StartingImage,
    /// Subject's own left side, uploaded in the fixed `contextImage1` slot.
    Pixal3dLeftView,
    /// Rear view, uploaded in the fixed `contextImage2` slot.
    Pixal3dBackView,
    /// Subject's own right side, uploaded in the fixed `contextImage3` slot.
    Pixal3dRightView,
    ControlNetImage,
    ContextImage(u8),
    ReferenceImage,
    ReferenceImageEnd,
    ReferenceAudio,
    /// Numbered MiniMax H3 r2v audio-reference slot (1 through 3).
    ReferenceAudioSlot(u8),
    ReferenceAudioIdentity,
    ReferenceVideo,
    /// Numbered MiniMax H3 r2v video-reference slot (1 through 3).
    ReferenceVideoSlot(u8),
    ReferenceMask,
    /// PNG alpha edit mask for the first GPT Image reference image.
    GptImageMask,
    Custom {
        name: String,
        media: bool,
    },
}

impl AssetRole {
    pub(super) fn wire_name(&self) -> String {
        match self {
            Self::StartingImage => "startingImage".into(),
            Self::Pixal3dLeftView => "contextImage1".into(),
            Self::Pixal3dBackView => "contextImage2".into(),
            Self::Pixal3dRightView => "contextImage3".into(),
            Self::ControlNetImage => "cnImage".into(),
            Self::ContextImage(index) => format!("contextImage{index}"),
            Self::ReferenceImage => "referenceImage".into(),
            Self::ReferenceImageEnd => "referenceImageEnd".into(),
            Self::ReferenceAudio | Self::ReferenceAudioIdentity => "referenceAudio".into(),
            Self::ReferenceAudioSlot(index) => format!("referenceAudio{index}"),
            Self::ReferenceVideo => "referenceVideo".into(),
            Self::ReferenceVideoSlot(index) => format!("referenceVideo{index}"),
            Self::ReferenceMask | Self::GptImageMask => "referenceMask".into(),
            Self::Custom { name, .. } => name.clone(),
        }
    }

    pub(super) fn param_name(&self) -> Option<String> {
        match self {
            Self::ControlNetImage => None,
            Self::Pixal3dLeftView => Some("leftViewImage".into()),
            Self::Pixal3dBackView => Some("backViewImage".into()),
            Self::Pixal3dRightView => Some("rightViewImage".into()),
            Self::GptImageMask => Some("gptImageMask".into()),
            Self::ContextImage(index) => Some(format!("contextImage{index}")),
            Self::Custom { name, .. } => Some(name.clone()),
            Self::ReferenceAudioIdentity => Some("referenceAudioIdentity".into()),
            _ => Some(self.wire_name()),
        }
    }

    pub(super) fn is_media(&self) -> bool {
        matches!(
            self,
            Self::ReferenceAudio
                | Self::ReferenceAudioSlot(_)
                | Self::ReferenceAudioIdentity
                | Self::ReferenceVideo
                | Self::ReferenceVideoSlot(_)
        ) || matches!(self, Self::Custom { media: true, .. })
    }
}

/// Extensible project request with ergonomic constructors for core generation.
#[derive(Clone, Debug)]
pub struct ProjectRequest {
    pub(super) params: Map<String, Value>,
    pub(super) assets: Vec<(AssetRole, MediaSource)>,
    pub(super) attribution: Option<WorkloadAttribution>,
}

pub(super) struct LoadedMedia {
    pub(super) data: Bytes,
    pub(super) file_name: String,
    pub(super) content_type: Option<String>,
}
