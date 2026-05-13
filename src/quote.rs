//! Port of `quote2sticker` — text quote rendered onto a colored background.
//!
//! Pure-Rust glyph rasterization via `ab_glyph`. Primary face is JMH
//! Typewriter Bold (the same slab/typewriter face the Python original used);
//! Noto Sans / Sans SC / Emoji are fallbacks for whatever JMH lacks
//! (Cyrillic, CJK, emoji). All Noto fonts are variable and dialled to weight
//! 700 at load time so the fallback weight matches.
//!
//! Note: JMH Typewriter is licensed for personal use only — switch the
//! primary face if you need to ship a publicly-licensed binary.

use std::sync::LazyLock;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, VariableFont};
use image::{ImageBuffer, ImageEncoder, Rgba, RgbaImage};

use crate::exceptions::{ProcessingError, ProcessingErrorKind};

const JMH_TYPEWRITER: &[u8] = include_bytes!("../assets/JMHTypewriter-Bold.ttf");
const NOTO_SANS: &[u8] = include_bytes!("../assets/NotoSans-Bold.ttf");
const NOTO_SANS_SC: &[u8] = include_bytes!("../assets/NotoSansSC-Bold.ttf");
const NOTO_EMOJI: &[u8] = include_bytes!("../assets/NotoEmoji-Bold.ttf");

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

fn load(bytes: &[u8], variable: bool) -> FontVec {
    let mut f = FontVec::try_from_vec(bytes.to_vec()).expect("bundled font is valid");
    if variable {
        // wght=700 = Bold. Some of our fonts also expose wdth; leave default.
        let _ = f.set_variation(b"wght", 700.0);
    }
    f
}

/// Font fallback chain. First match wins per glyph. JMH is the look; Noto
/// fills the gaps (no Cyrillic / CJK / emoji in JMH).
static FONTS: LazyLock<Vec<FontVec>> = LazyLock::new(|| {
    vec![
        load(JMH_TYPEWRITER, false),
        load(NOTO_SANS, true),
        load(NOTO_SANS_SC, true),
        load(NOTO_EMOJI, true),
    ]
});

/// Pick the first font in the chain that has a glyph for `ch`. Returns the
/// primary if none match (so we still draw .notdef tofu, not nothing).
fn font_for(ch: char) -> &'static FontVec {
    for f in FONTS.iter() {
        if f.glyph_id(ch).0 != 0 {
            return f;
        }
    }
    &FONTS[0]
}

fn h_advance(ch: char, scale: PxScale) -> f32 {
    let f = font_for(ch);
    f.as_scaled(scale).h_advance(f.glyph_id(ch))
}

fn measure(text: &str, scale: PxScale) -> f32 {
    text.chars().map(|c| h_advance(c, scale)).sum()
}

fn line_height(scale: PxScale) -> f32 {
    // All our fonts are Noto family with consistent metrics; use the primary.
    let sf = FONTS[0].as_scaled(scale);
    sf.ascent() - sf.descent() + sf.line_gap()
}

/// Wrap text so each line fits within `width / 1.618` (matches Python's golden
/// ratio heuristic).
fn wrap(text: &str, scale: PxScale, width: u32) -> Vec<String> {
    let limit = (width as f32) / 1.618;
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w = 0.0_f32;
    for ch in text.chars() {
        if ch == '\n' {
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
            continue;
        }
        let glyph_w = h_advance(ch, scale);
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
    scale: PxScale,
    ch: char,
    pen_x: f32,
    baseline_y: f32,
    fill: Rgba<u8>,
) -> f32 {
    let font = font_for(ch);
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
            let a = ((cov * fill[3] as f32).round() as u32).min(255);
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
    let scale = PxScale::from(DEFAULT_FONT_SIZE);

    let (r, g, b) = COLORS[color_index % COLORS.len()];
    let mut canvas: RgbaImage =
        ImageBuffer::from_pixel(QUOTE_WIDTH, QUOTE_HEIGHT, Rgba([r, g, b, 255]));

    let mut body = wrap(&truncated, scale, QUOTE_WIDTH);
    body.push(String::new());
    body.push(format!("— {author}"));

    let lh = line_height(scale);
    let total_h = lh * body.len() as f32;
    let start_y = (QUOTE_HEIGHT as f32 - total_h) / 2.0;
    let baseline_offset = FONTS[0].as_scaled(scale).ascent();

    for (i, line) in body.iter().enumerate() {
        let line_w = measure(line, scale);
        let mut pen_x = (QUOTE_WIDTH as f32 - line_w) / 2.0;
        let baseline_y = start_y + i as f32 * lh + baseline_offset;
        for ch in line.chars() {
            pen_x += draw_glyph(
                &mut canvas,
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
