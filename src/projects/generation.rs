use super::*;

/// Single-view Pixal3D graph selector. Omit to use the worker's shipped default.
/// The multi-view model accepts no graph selector.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Pixal3dTemplateVariant {
    #[serde(rename = "i23d-birefnet")]
    I23dBirefnet,
}

/// Pixal3D options shared by both image-to-3D workflows. Smaller textures and
/// mesh targets reduce work; `shape_resolution` 1536 is a priced increase from 1024.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pixal3dGenerationOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_variant: Option<Pixal3dTemplateVariant>,
    /// Base-color texture resolution, 1024–4096.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture_size: Option<u32>,
    /// Triangle target, 5000–700000.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh_target_faces: Option<u32>,
    /// Normal-map resolution, 512–2048.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal_map_size: Option<u32>,
    /// Ambient-occlusion resolution, 256–1024.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambient_occlusion_size: Option<u32>,
    /// Sparse-latent resolution, 1024–1536.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape_resolution: Option<u32>,
}
