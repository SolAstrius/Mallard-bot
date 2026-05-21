//! Byte-exact golden tests for the image pipeline.
//!
//! Set `UPDATE_GOLDENS=1` when running `cargo test --test golden` to regenerate
//! the fixtures in `tests/fixtures/`. CI runs in compare mode and fails on any
//! pixel drift.

use std::fs;
use std::path::{Path, PathBuf};

use image::{ImageBuffer, ImageEncoder, Rgba, RgbaImage};
use mallard_bot::arguments::PhotoQuoteArguments;
use mallard_bot::imaging::{image_to_emoji, image_to_sticker, FilePreprocessType};
use mallard_bot::mask::Mask;
use mallard_bot::quote::render_quote;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn assert_golden(name: &str, actual: &[u8]) {
    let path = fixtures_dir().join(name);
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::create_dir_all(fixtures_dir()).unwrap();
        fs::write(&path, actual).unwrap();
        eprintln!("updated golden: {}", path.display());
        return;
    }
    let expected = fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}. Run `UPDATE_GOLDENS=1 cargo test --test golden` to seed.",
            path.display()
        )
    });
    if expected != actual {
        let actual_img = image::load_from_memory(actual).unwrap().to_rgba8();
        let expected_img = image::load_from_memory(&expected).unwrap().to_rgba8();
        assert_eq!(
            actual_img.dimensions(),
            expected_img.dimensions(),
            "dimensions differ for {name}"
        );
        let mut max_delta = 0i32;
        let mut diff_count = 0usize;
        for (a, e) in actual_img.pixels().zip(expected_img.pixels()) {
            for i in 0..4 {
                let d = (a[i] as i32 - e[i] as i32).abs();
                if d > 0 {
                    diff_count += 1;
                    if d > max_delta {
                        max_delta = d;
                    }
                }
            }
        }
        panic!(
            "golden mismatch for {name}: differing_components={diff_count}, max_delta={max_delta}"
        );
    }
}

fn solid(w: u32, h: u32, p: [u8; 4]) -> RgbaImage {
    ImageBuffer::from_pixel(w, h, Rgba(p))
}

fn to_png(img: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    out
}

#[test]
fn golden_image_to_emoji_solid_red() {
    let src = solid(200, 200, [220, 30, 40, 255]);
    let out = image_to_emoji(&to_png(&src)).unwrap();
    assert_golden("emoji_red_200.png", &out);
}

#[test]
fn golden_image_to_sticker_circle_solid_green() {
    let src = solid(640, 640, [60, 180, 90, 255]);
    let out = image_to_sticker(
        &to_png(&src),
        FilePreprocessType::Mask(Mask::Circle),
        &PhotoQuoteArguments::default(),
    )
    .unwrap();
    assert_golden("sticker_circle_green.png", &out);
}

#[test]
fn golden_image_to_sticker_default_with_bubble() {
    let src = solid(800, 400, [30, 30, 30, 255]);
    let args = PhotoQuoteArguments {
        speech_bubble: Some(0),
        ..Default::default()
    };
    let out = image_to_sticker(&to_png(&src), FilePreprocessType::Default, &args).unwrap();
    assert_golden("sticker_bubble_right.png", &out);
}

#[test]
fn golden_quote_basic() {
    let out = render_quote("Кря-кря, мир!", "Mallard", 0).unwrap();
    assert_golden("quote_basic.png", &out);
}

#[test]
fn golden_quote_color_index_2() {
    let out = render_quote("Hello", "Author", 2).unwrap();
    assert_golden("quote_purple.png", &out);
}
