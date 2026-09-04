mod edit;
mod text;

use anyhow::{Result, bail};

pub use edit::edit_model;
pub use text::{TEXT_MODELS, text_model};

#[derive(Clone, Copy, Debug)]
pub struct ImageModelSpec {
    pub key: &'static str,
    pub id: &'static str,
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub min_steps: u32,
    pub max_steps: u32,
    pub default_steps: u32,
    pub min_guidance: f64,
    pub max_guidance: f64,
    pub default_guidance: f64,
    pub sampler: &'static str,
    pub scheduler: &'static str,
    pub negative_prompt: Option<&'static str>,
    pub supports_starting_image: bool,
    pub max_context_images: usize,
}

pub fn validate_image_options(
    model: ImageModelSpec,
    width: u32,
    height: u32,
    steps: u32,
    guidance: f64,
) -> Result<()> {
    if width == 0 || height == 0 || width > model.max_width || height > model.max_height {
        bail!(
            "{} dimensions must be positive and no larger than {}x{}",
            model.name,
            model.max_width,
            model.max_height
        );
    }
    if !(model.min_steps..=model.max_steps).contains(&steps) {
        bail!(
            "{} steps must be between {} and {}",
            model.name,
            model.min_steps,
            model.max_steps
        );
    }
    if !(model.min_guidance..=model.max_guidance).contains(&guidance) {
        bail!(
            "{} guidance must be between {} and {}",
            model.name,
            model.min_guidance,
            model.max_guidance
        );
    }
    Ok(())
}
