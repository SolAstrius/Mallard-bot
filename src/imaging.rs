//! Native-Rust port of stickers.py's image pipeline.
//!
//! `image` + `imageproc` replace the OpenCV/numpy work end-to-end. Output is
//! always an in-memory PNG (`Vec<u8>`), the same shape the Python returned
//! via `BytesIO`.

use std::io::Cursor;

use image::imageops::FilterType;
use image::{ImageBuffer, ImageEncoder, ImageReader, Rgba, RgbaImage};

use crate::arguments::{PhotoQuoteArguments, BUBBLE_NAMES};
use crate::exceptions::{ProcessingError, ProcessingErrorKind};
use crate::mask::{self, Mask};

/// Mirrors the Python `FilePreprocessType` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilePreprocessType {
    #[default]
    Default,
    Mask(Mask),
    VideoThumb,
    Animation,
}

const STICKER_SIZE: u32 = 512;
const EMOJI_SIZE: u32 = 100;

pub fn desired_size(is_emoji: bool) -> u32 {
    if is_emoji {
        EMOJI_SIZE
    } else {
        STICKER_SIZE
    }
}

fn err_unexpected(msg: impl Into<String>) -> ProcessingError {
    ProcessingError::new(ProcessingErrorKind::Unexpected, msg)
}

fn decode_rgba(bytes: &[u8]) -> Result<RgbaImage, ProcessingError> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| err_unexpected(format!("format detect: {e}")))?;
    let dynamic = reader
        .decode()
        .map_err(|e| err_unexpected(format!("decode: {e}")))?;
    Ok(dynamic.to_rgba8())
}

fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, ProcessingError> {
    let mut out = Vec::with_capacity((img.width() * img.height() * 4) as usize);
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    encoder
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| err_unexpected(format!("encode: {e}")))?;
    Ok(out)
}

/// Resize the longest edge to `desired`, preserving aspect ratio. Lanczos3 to
/// keep the result sharp at sticker sizes.
pub fn fit_longest_edge(img: &RgbaImage, desired: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let (new_w, new_h) = if w >= h {
        (
            desired,
            (h as f64 * (desired as f64 / w as f64)).round() as u32,
        )
    } else {
        (
            (w as f64 * (desired as f64 / h as f64)).round() as u32,
            desired,
        )
    };
    image::imageops::resize(img, new_w.max(1), new_h.max(1), FilterType::Lanczos3)
}

/// Anti-aliased circular alpha mask sized `(size, size)`. Pixels inside the
/// circle are opaque white; outside fully transparent; the 1-px border ring is
/// alpha-blended for an AA edge.
pub fn circular_mask(size: u32) -> RgbaImage {
    let mut mask: RgbaImage = ImageBuffer::from_pixel(size, size, Rgba([0, 0, 0, 0]));
    let cx = (size as f32 - 1.0) / 2.0;
    let cy = (size as f32 - 1.0) / 2.0;
    // Match Python's `int(size * 0.976) // 2` to keep the same crop ratio.
    let radius = (((size as f32) * 0.976) as u32 / 2) as f32;
    for (x, y, p) in mask.enumerate_pixels_mut() {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let d = (dx * dx + dy * dy).sqrt();
        let alpha = if d <= radius - 0.5 {
            255.0
        } else if d >= radius + 0.5 {
            0.0
        } else {
            (radius + 0.5 - d) * 255.0
        };
        *p = Rgba([255, 255, 255, alpha.round().clamp(0.0, 255.0) as u8]);
    }
    mask
}

/// Apply the circular mask to `img` in-place: alpha channel becomes the
/// product of the source alpha and the mask alpha.
pub fn apply_mask(img: &mut RgbaImage, mask: &RgbaImage) {
    debug_assert_eq!(img.dimensions(), mask.dimensions());
    for (p, m) in img.pixels_mut().zip(mask.pixels()) {
        let src_a = p[3] as u16;
        let mask_a = m[3] as u16;
        p[3] = ((src_a * mask_a) / 255) as u8;
    }
}

/// Source-over compositing of `top` onto `base` (premultiplied math, straight
/// alpha in/out). Dimensions must match.
pub fn over(base: &mut RgbaImage, top: &RgbaImage) {
    debug_assert_eq!(base.dimensions(), top.dimensions());
    for (b, t) in base.pixels_mut().zip(top.pixels()) {
        let ta = t[3] as u32;
        if ta == 0 {
            continue;
        }
        if ta == 255 {
            *b = *t;
            continue;
        }
        let ba = b[3] as u32;
        let inv = 255 - ta;
        let out_a = ta + (ba * inv) / 255;
        if out_a == 0 {
            *b = Rgba([0, 0, 0, 0]);
            continue;
        }
        for c in 0..3 {
            let bc = b[c] as u32;
            let tc = t[c] as u32;
            let num = tc * ta * 255 + bc * ba * inv;
            b[c] = (num / (out_a * 255)) as u8;
        }
        b[3] = out_a as u8;
    }
}

/// Load and resize a bubble overlay PNG to `(w, h)`.
pub fn load_bubble(idx: usize, w: u32, h: u32) -> Result<RgbaImage, ProcessingError> {
    let path = BUBBLE_NAMES
        .get(idx)
        .ok_or_else(|| err_unexpected(format!("bubble index {idx} out of range")))?;
    let img = image::open(path).map_err(|e| err_unexpected(format!("open bubble: {e}")))?;
    Ok(image::imageops::resize(
        &img.to_rgba8(),
        w,
        h,
        FilterType::Lanczos3,
    ))
}

/// 100×100 PNG of the source image — port of `image2emoji`.
pub fn image_to_emoji(bytes: &[u8]) -> Result<Vec<u8>, ProcessingError> {
    let img = decode_rgba(bytes)?;
    let resized = image::imageops::resize(&img, EMOJI_SIZE, EMOJI_SIZE, FilterType::Lanczos3);
    encode_png(&resized)
}

/// Image → sticker PNG. Port of `file2sticker` for the non-video paths.
pub fn image_to_sticker(
    bytes: &[u8],
    preprocess: FilePreprocessType,
    args: &PhotoQuoteArguments,
) -> Result<Vec<u8>, ProcessingError> {
    let mut img = decode_rgba(bytes)?;
    let target = desired_size(args.is_emoji.unwrap_or(false));

    img = match preprocess {
        FilePreprocessType::Mask(m) => {
            let (w, h) = img.dimensions();
            let side = w.min(h);
            let x = (w - side) / 2;
            let y = (h - side) / 2;
            let square = image::imageops::crop(&mut img, x, y, side, side).to_image();
            let mut out = image::imageops::resize(&square, target, target, FilterType::Lanczos3);
            apply_mask(&mut out, &mask::render(m, target));
            out
        }
        FilePreprocessType::Default
        | FilePreprocessType::VideoThumb
        | FilePreprocessType::Animation => fit_longest_edge(&img, target),
    };

    if let Some(idx) = args.speech_bubble {
        let (w, h) = img.dimensions();
        let bubble = load_bubble(idx, w, h)?;
        over(&mut img, &bubble);
    }

    encode_png(&img)
}
