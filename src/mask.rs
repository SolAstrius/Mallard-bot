//! Procedural mask shapes for `/qva` stickers.
//!
//! Each variant renders to an RGBA image where opaque white = inside the
//! shape, transparent = outside. The output is always square (`size × size`)
//! and the shape is centered with the same visual padding as the original
//! `imaging::circular_mask` (0.976 of the half-edge).

use image::{ImageBuffer, Rgba, RgbaImage};

use crate::imaging::circular_mask;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mask {
    Circle,
    Square,
    Triangle,
    Diamond,
    Hexagon,
    Star,
}

impl Mask {
    pub fn from_preset(name: &str) -> Option<Self> {
        Some(match name {
            "circle" => Mask::Circle,
            "square" => Mask::Square,
            "triangle" => Mask::Triangle,
            "diamond" => Mask::Diamond,
            "hexagon" => Mask::Hexagon,
            "star" => Mask::Star,
            _ => return None,
        })
    }
}

/// Render the mask at `size × size`. Circle keeps its bespoke per-pixel
/// distance AA; other shapes are rasterized with 3×3 supersampling.
pub fn render(mask: Mask, size: u32) -> RgbaImage {
    if matches!(mask, Mask::Circle) {
        return circular_mask(size);
    }
    let mut out: RgbaImage = ImageBuffer::from_pixel(size, size, Rgba([0, 0, 0, 0]));
    // Same 0.976 padding factor as the circular mask so all shapes feel
    // visually consistent and leave a 1-2px guard against the sticker edge.
    let scale = (size as f32) * 0.976 / 2.0;
    let cx = (size as f32 - 1.0) / 2.0;
    let cy = (size as f32 - 1.0) / 2.0;
    const SS: u32 = 3;
    let step = 1.0 / SS as f32;
    let base = -0.5 + step / 2.0;

    for (px, py, pixel) in out.enumerate_pixels_mut() {
        let mut hits = 0u32;
        for sy in 0..SS {
            for sx in 0..SS {
                let sxf = base + sx as f32 * step;
                let syf = base + sy as f32 * step;
                let x = (px as f32 + sxf - cx) / scale;
                // Flip Y so positive points up (math convention) — makes the
                // triangle apex go up, star top point up, etc.
                let y = -((py as f32 + syf - cy) / scale);
                if inside(mask, x, y) {
                    hits += 1;
                }
            }
        }
        let alpha = ((hits * 255) / (SS * SS)) as u8;
        *pixel = Rgba([255, 255, 255, alpha]);
    }
    out
}

fn inside(mask: Mask, x: f32, y: f32) -> bool {
    match mask {
        Mask::Circle => x * x + y * y <= 1.0,
        Mask::Square => x.abs() <= 1.0 && y.abs() <= 1.0,
        Mask::Diamond => x.abs() + y.abs() <= 1.0,
        Mask::Triangle => {
            // Equilateral, apex up, inscribed in the unit circle.
            let s3 = 3.0_f32.sqrt();
            y >= -0.5 && y <= s3 * x + 1.0 && y <= -s3 * x + 1.0
        }
        Mask::Hexagon => {
            // Pointy-top regular hexagon inscribed in the unit circle.
            // Vertical edges at x = ±√3/2; slanted edges given by |y| + |x|/√3 ≤ 1.
            let s3 = 3.0_f32.sqrt();
            x.abs() <= s3 / 2.0 && y.abs() + x.abs() / s3 <= 1.0
        }
        Mask::Star => {
            // 5-pointed star, outer radius 1, inner radius via golden-ratio
            // construction so points look classic-pentagram-ish.
            const RI: f32 = 0.381_966; // sin(18°)/sin(54°)
            let mut poly = [(0.0_f32, 0.0_f32); 10];
            for (k, slot) in poly.iter_mut().enumerate() {
                let r = if k % 2 == 0 { 1.0 } else { RI };
                let theta = std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
                *slot = (r * theta.cos(), r * theta.sin());
            }
            point_in_polygon(x, y, &poly)
        }
    }
}

fn point_in_polygon(x: f32, y: f32, poly: &[(f32, f32)]) -> bool {
    let n = poly.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) {
            let x_cross = xi + (y - yi) * (xj - xi) / (yj - yi);
            if x < x_cross {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}
