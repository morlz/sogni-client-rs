use anyhow::{Result, bail};

use super::ImageModelSpec;

pub fn edit_model(key: &str) -> Result<ImageModelSpec> {
    let model = match key {
        "qwen-lightning" => edit(
            "qwen-lightning",
            "qwen_image_edit_2511_fp8_lightning",
            "Qwen Image Edit 2511 Lightning",
            2560,
            4,
            8,
            4,
            0.6,
            1.6,
            1.0,
            3,
        ),
        "qwen" => edit(
            "qwen",
            "qwen_image_edit_2511_fp8",
            "Qwen Image Edit 2511",
            2560,
            20,
            50,
            20,
            2.5,
            5.0,
            4.0,
            3,
        ),
        "krea-identity-edit" => edit(
            "krea-identity-edit",
            "krea2_identity_edit_v1_2",
            "Krea 2 Identity Edit v1.2",
            2048,
            8,
            12,
            10,
            0.6,
            1.6,
            1.0,
            2,
        ),
        "dark-beast-krea2-identity-edit" => edit(
            "dark-beast-krea2-identity-edit",
            "dark_beast_krea2_identity_edit_v1_2",
            "Dark Beast Krea 2 Identity Edit",
            2048,
            8,
            12,
            10,
            0.6,
            1.6,
            1.0,
            2,
        ),
        _ => bail!("unknown image-edit model key {key}"),
    };
    Ok(model)
}

#[allow(clippy::too_many_arguments)]
const fn edit(
    key: &'static str,
    id: &'static str,
    name: &'static str,
    maximum: u32,
    min_steps: u32,
    max_steps: u32,
    default_steps: u32,
    min_guidance: f64,
    max_guidance: f64,
    default_guidance: f64,
    max_context_images: usize,
) -> ImageModelSpec {
    ImageModelSpec {
        key,
        id,
        name,
        width: 1024,
        height: 1024,
        max_width: maximum,
        max_height: maximum,
        min_steps,
        max_steps,
        default_steps,
        min_guidance,
        max_guidance,
        default_guidance,
        sampler: "euler",
        scheduler: "simple",
        negative_prompt: None,
        supports_starting_image: false,
        max_context_images,
    }
}
