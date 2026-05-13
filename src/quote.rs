//! Port of `quote2sticker` — text quote rendered onto a colored background.
//!
//! Pillow does anti-aliased glyph rasterization through FreeType; we use the
//! pure-Rust `ab_glyph` rasterizer, so pixel output won't match PIL exactly.
//! It is, however, deterministic for a given (text, author, color_index) so
//! per-pixel golden tests are meaningful.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{ImageBuffer, ImageEncoder, Rgba, RgbaImage};

use crate::exceptions::{ProcessingError, ProcessingErrorKind};

const FONT_BYTES: &[u8] = include_bytes!("../assets/LiberationSans-Bold.ttf");

const COLORS: [(u8, u8, u8); 4] = [
    (248, 205, 48),
    (75, 151, 75),
    (150, 112, 159),
    (211, 111, 76),
];

pub const QUOTE_WIDTH: u32 = 512;
pub const QUOTE_HEIGHT: u32 = 384;
const DEFAULT_FONT_SIZE: f32 = 28.0;

fn err(msg: impl Into<String>) -> ProcessingError {
    ProcessingError::new(ProcessingErrorKind::Unexpected, msg)
}

fn load_font() -> Result<FontRef<'static>, ProcessingError> {
    FontRef::try_from_slice(FONT_BYTES).map_err(|e| err(format!("font: {e}")))
}

fn measure(font: &FontRef, scale: PxScale, text: &str) -> f32 {
    let sf = font.as_scaled(scale);
    text.chars()
        .map(|c| sf.h_advance(font.glyph_id(c)))
        .sum::<f32>()
}

fn line_height(font: &FontRef, scale: PxScale) -> f32 {
    let sf = font.as_scaled(scale);
    sf.ascent() - sf.descent() + sf.line_gap()
}

/// Wrap text so each line fits within `width / 1.618` (matches Python's golden
/// ratio heuristic).
fn wrap(text: &str, font: &FontRef, scale: PxScale, width: u32) -> Vec<String> {
    let limit = (width as f32) / 1.618;
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w = 0.0_f32;
    for ch in text.chars() {
        let glyph_w = font.as_scaled(scale).h_advance(font.glyph_id(ch));
        if ch == '\n' {
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
            continue;
        }
        if current_w + glyph_w > limit && ch == ' ' {
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
        } else {
            current.push(ch);
            current_w += glyph_w;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn draw_glyph(
    canvas: &mut RgbaImage,
    font: &FontRef,
    scale: PxScale,
    ch: char,
    pen_x: f32,
    baseline_y: f32,
    fill: Rgba<u8>,
) -> f32 {
    let sf = font.as_scaled(scale);
    let glyph = font
        .glyph_id(ch)
        .with_scale_and_position(scale, ab_glyph::point(pen_x, baseline_y));
    if let Some(outlined) = font.outline_glyph(glyph) {
        let bb = outlined.px_bounds();
        outlined.draw(|x, y, cov| {
            let px = bb.min.x as i32 + x as i32;
            let py = bb.min.y as i32 + y as i32;
            if px < 0 || py < 0 || px >= canvas.width() as i32 || py >= canvas.height() as i32 {
                return;
            }
            let dst = canvas.get_pixel_mut(px as u32, py as u32);
            let a = (cov * fill[3] as f32).round() as u32;
            let ia = 255 - a;
            for c in 0..3 {
                dst[c] = (((dst[c] as u32) * ia + (fill[c] as u32) * a) / 255) as u8;
            }
            dst[3] = 255;
        });
    }
    sf.h_advance(font.glyph_id(ch))
}

pub fn render_quote(
    quote_text: &str,
    author: &str,
    color_index: usize,
) -> Result<Vec<u8>, ProcessingError> {
    let truncated: String = quote_text.chars().take(181).collect();
    let font = load_font()?;
    let scale = PxScale::from(DEFAULT_FONT_SIZE);

    let (r, g, b) = COLORS[color_index % COLORS.len()];
    let mut canvas: RgbaImage =
        ImageBuffer::from_pixel(QUOTE_WIDTH, QUOTE_HEIGHT, Rgba([r, g, b, 255]));

    let mut body = wrap(&truncated, &font, scale, QUOTE_WIDTH);
    body.push(String::new());
    body.push(format!("— {author}"));

    let lh = line_height(&font, scale);
    let total_h = lh * body.len() as f32;
    let start_y = (QUOTE_HEIGHT as f32 - total_h) / 2.0;
    let baseline_offset = font.as_scaled(scale).ascent();

    for (i, line) in body.iter().enumerate() {
        let line_w = measure(&font, scale, line);
        let mut pen_x = (QUOTE_WIDTH as f32 - line_w) / 2.0;
        let baseline_y = start_y + i as f32 * lh + baseline_offset;
        for ch in line.chars() {
            pen_x += draw_glyph(
                &mut canvas,
                &font,
                scale,
                ch,
                pen_x,
                baseline_y,
                Rgba([0, 0, 0, 255]),
            );
        }
    }

    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            canvas.as_raw(),
            QUOTE_WIDTH,
            QUOTE_HEIGHT,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| err(format!("encode: {e}")))?;
    Ok(out)
}
