//! GPT Image 2.6 frame sizing for animated GIF requests.
use crate::{Error, Result};

const MAX_EDGE: u32 = 3840;
const MIN_PIXELS: u32 = 655_360;
const MAX_PIXELS: u32 = 8_294_400;
const DIMENSION_STEP: u32 = 16;
const MAX_ASPECT: u32 = 3;

/// A supported sheet layout, including any unused cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GptImageGifLayout {
    pub columns: u32,
    pub rows: u32,
}

/// Frame dimensions and area limits for the requested number of frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GptImageGifFrameBounds {
    pub min_dimension: u32,
    pub dimension_step: u32,
    pub max_aspect_ratio: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub min_pixels: u32,
    pub max_pixels: u32,
}

/// Sheet geometry for a concrete frame size and count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GptImageGifSheet {
    pub columns: u32,
    pub rows: u32,
    pub width: u32,
    pub height: u32,
}

/// Return supported sheet layouts for 2–16 frames (default 9).
pub fn get_gpt_image_gif_layouts(frame_count: Option<u32>) -> Result<&'static [GptImageGifLayout]> {
    const fn layout(columns: u32, rows: u32) -> GptImageGifLayout {
        GptImageGifLayout { columns, rows }
    }
    const EIGHT: &[GptImageGifLayout] = &[layout(4, 2), layout(2, 4)];
    const NINE: &[GptImageGifLayout] = &[layout(3, 3)];
    const TWELVE: &[GptImageGifLayout] = &[layout(4, 3), layout(3, 4)];
    const SIXTEEN: &[GptImageGifLayout] = &[layout(4, 4)];
    match frame_count.unwrap_or(9) {
        2..=8 => Ok(EIGHT),
        9 => Ok(NINE),
        10..=12 => Ok(TWELVE),
        13..=16 => Ok(SIXTEEN),
        _ => Err(Error::InvalidInput("GIF frame count must be 2–16".into())),
    }
}

/// Return the frame bounds implied by the supported sheet layouts.
pub fn get_gpt_image_gif_frame_bounds(frame_count: Option<u32>) -> Result<GptImageGifFrameBounds> {
    let layouts = get_gpt_image_gif_layouts(frame_count)?;
    let cells = layouts[0].columns * layouts[0].rows;
    Ok(GptImageGifFrameBounds {
        min_dimension: DIMENSION_STEP,
        dimension_step: DIMENSION_STEP,
        max_aspect_ratio: MAX_ASPECT,
        max_width: layouts
            .iter()
            .map(|layout| MAX_EDGE / layout.columns)
            .max()
            .unwrap(),
        max_height: layouts
            .iter()
            .map(|layout| MAX_EDGE / layout.rows)
            .max()
            .unwrap(),
        min_pixels: MIN_PIXELS.div_ceil(cells),
        max_pixels: MAX_PIXELS / cells,
    })
}

/// Validate concrete frame pixels and return the closest-to-square valid sheet.
pub fn get_gpt_image_gif_sheet(
    width: u32,
    height: u32,
    frame_count: Option<u32>,
) -> Result<GptImageGifSheet> {
    let layouts = get_gpt_image_gif_layouts(frame_count)?;
    if width == 0
        || height == 0
        || !width.is_multiple_of(DIMENSION_STEP)
        || !height.is_multiple_of(DIMENSION_STEP)
        || !valid_aspect(width, height)
    {
        return Err(Error::InvalidInput(
            "GIF frame dimensions must be positive multiples of 16 within 3:1".into(),
        ));
    }
    layouts
        .iter()
        .filter(|layout| width <= MAX_EDGE / layout.columns && height <= MAX_EDGE / layout.rows)
        .map(|layout| GptImageGifSheet {
            columns: layout.columns,
            rows: layout.rows,
            width: width * layout.columns,
            height: height * layout.rows,
        })
        .filter(|sheet| {
            valid_aspect(sheet.width, sheet.height)
                && (MIN_PIXELS..=MAX_PIXELS).contains(&(sheet.width * sheet.height))
        })
        .min_by(|a, b| {
            (a.width.max(a.height) * b.width.min(b.height))
                .cmp(&(b.width.max(b.height) * a.width.min(a.height)))
        })
        .ok_or_else(|| Error::InvalidInput(
            "GIF frames cannot fit a supported sheet: use smaller or larger frames within the 655360–8294400 pixel budget and 3840 sheet edge limit".into(),
        ))
}

fn valid_aspect(width: u32, height: u32) -> bool {
    u64::from(width.max(height)) <= u64::from(width.min(height)) * u64::from(MAX_ASPECT)
}

/// Preserve explicit frame dimensions and infer omitted values within the
/// sheet limits. The optional aspect ratio (width / height) defaults to 1.
pub fn resolve_gpt_image_gif_size(
    width: Option<u32>,
    height: Option<u32>,
    frame_count: Option<u32>,
    ratio: Option<f64>,
) -> Result<(u32, u32)> {
    let layouts = get_gpt_image_gif_layouts(frame_count)?;
    let ratio = ratio.unwrap_or(1.0);
    let limit = f64::from(MAX_ASPECT);
    if !(1.0 / limit..=limit).contains(&ratio) {
        return Err(Error::InvalidInput(
            "GIF aspect ratio must be within 3:1".into(),
        ));
    }
    let step = f64::from(DIMENSION_STEP);
    let round_dimension = |dimension: f64| ((dimension / step).round() * step).max(step) as u32;
    if width.is_some() || height.is_some() {
        let width =
            width.unwrap_or_else(|| round_dimension(f64::from(height.unwrap_or(0)) * ratio));
        let height = height.unwrap_or_else(|| round_dimension(f64::from(width) / ratio));
        get_gpt_image_gif_sheet(width, height, frame_count)?;
        return Ok((width, height));
    }
    let mut best = None;
    for layout in layouts {
        let (columns, rows) = (f64::from(layout.columns), f64::from(layout.rows));
        if !(1.0 / limit..=limit).contains(&(ratio * columns / rows)) {
            continue;
        }
        let max_edge = f64::from(MAX_EDGE);
        let area = 896.0_f64
            .powi(2)
            .min(f64::from(MAX_PIXELS) / (columns * rows))
            .min((max_edge / columns).powi(2) / ratio)
            .min((max_edge / rows).powi(2) * ratio);
        let mut width = ((area * ratio).sqrt() / step).floor() as u32 * DIMENSION_STEP;
        let mut height = ((area / ratio).sqrt() / step).floor() as u32 * DIMENSION_STEP;
        width = width.min(height * MAX_ASPECT);
        height = height.min(width * MAX_ASPECT);
        get_gpt_image_gif_sheet(width, height, frame_count)?;
        if best.is_none_or(|(w, h)| width * height > w * h) {
            best = Some((width, height));
        }
    }
    best.ok_or_else(|| Error::InvalidInput("GIF aspect ratio cannot fit a supported sheet".into()))
}
