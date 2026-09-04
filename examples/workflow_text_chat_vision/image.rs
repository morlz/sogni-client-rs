use std::{fs, io::Cursor, path::Path};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::ImageFormat;

const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_SDK_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone)]
pub struct LoadedImage {
    pub data_uri: String,
    pub file_name: String,
    pub source_bytes: u64,
    pub format: &'static str,
}

pub fn load(path: impl AsRef<Path>) -> Result<LoadedImage> {
    let path = path.as_ref();
    let metadata = fs::metadata(path)
        .with_context(|| format!("image not found or unreadable: {}", path.display()))?;
    if !metadata.is_file() {
        bail!("image path is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        bail!(
            "image is {:.1} MiB; source limit is 20 MiB",
            metadata.len() as f64 / 1_048_576.0
        );
    }
    let bytes = fs::read(path).with_context(|| format!("read image: {}", path.display()))?;
    let guessed = image::guess_format(&bytes).context("detect image format from file contents")?;
    let (payload, mime, label) = match guessed {
        ImageFormat::Jpeg if bytes.len() <= MAX_SDK_BYTES => (bytes, "image/jpeg", "JPEG"),
        ImageFormat::Png if bytes.len() <= MAX_SDK_BYTES => (bytes, "image/png", "PNG"),
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP | ImageFormat::Gif => {
            let decoded = image::load_from_memory_with_format(&bytes, guessed)
                .context("decode JPEG, PNG, WebP, or first GIF frame")?;
            let mut encoded = Cursor::new(Vec::new());
            decoded
                .write_to(&mut encoded, ImageFormat::Png)
                .context("convert image to PNG for vision chat")?;
            let encoded = encoded.into_inner();
            if encoded.len() > MAX_SDK_BYTES {
                bail!(
                    "preprocessed PNG is {:.1} MiB; Sogni vision accepts at most 10 MiB. Resize the image first",
                    encoded.len() as f64 / 1_048_576.0
                );
            }
            let label = match guessed {
                ImageFormat::WebP => "WEBP->PNG",
                ImageFormat::Gif => "GIF(first frame)->PNG",
                ImageFormat::Jpeg => "JPEG->PNG",
                _ => "PNG",
            };
            (encoded, "image/png", label)
        }
        other => bail!(
            "unsupported image format {other:?}; supported formats are JPEG, PNG, WebP, and GIF"
        ),
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_owned();
    Ok(LoadedImage {
        data_uri: format!("data:{mime};base64,{}", STANDARD.encode(payload)),
        file_name,
        source_bytes: metadata.len(),
        format: label,
    })
}
