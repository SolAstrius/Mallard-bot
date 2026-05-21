//! Per-pixel coverage of the image pipeline.

use image::{ImageBuffer, ImageEncoder, Rgba, RgbaImage};
use mallard_bot::arguments::PhotoQuoteArguments;
use mallard_bot::imaging::{
    apply_mask, circular_mask, desired_size, fit_longest_edge, image_to_emoji, image_to_sticker,
    over, FilePreprocessType,
};

fn solid(w: u32, h: u32, pixel: [u8; 4]) -> RgbaImage {
    ImageBuffer::from_pixel(w, h, Rgba(pixel))
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

fn decode(bytes: &[u8]) -> RgbaImage {
    image::load_from_memory(bytes).unwrap().to_rgba8()
}

#[test]
fn desired_size_matches_python_constants() {
    assert_eq!(desired_size(false), 512);
    assert_eq!(desired_size(true), 100);
}

#[test]
fn fit_longest_edge_landscape() {
    let src = solid(400, 200, [10, 20, 30, 255]);
    let out = fit_longest_edge(&src, 512);
    assert_eq!(out.dimensions(), (512, 256));
}

#[test]
fn fit_longest_edge_portrait() {
    let src = solid(200, 400, [10, 20, 30, 255]);
    let out = fit_longest_edge(&src, 512);
    assert_eq!(out.dimensions(), (256, 512));
}

#[test]
fn image_to_emoji_resizes_exactly_to_100() {
    let red = solid(200, 200, [255, 0, 0, 255]);
    let bytes = image_to_emoji(&to_png(&red)).unwrap();
    let out = decode(&bytes);
    assert_eq!(out.dimensions(), (100, 100));
    // Solid red in, solid red out — every pixel should match within Lanczos
    // numerical tolerance.
    for p in out.pixels() {
        assert!((p[0] as i32 - 255).abs() <= 1);
        assert!(p[1] <= 1);
        assert!(p[2] <= 1);
        assert_eq!(p[3], 255);
    }
}

#[test]
fn circular_mask_corners_transparent_center_opaque() {
    let mask = circular_mask(64);
    let center = mask.get_pixel(32, 32);
    assert_eq!(center[3], 255, "center must be fully opaque");
    for &(x, y) in &[(0, 0), (63, 0), (0, 63), (63, 63)] {
        assert_eq!(
            mask.get_pixel(x, y)[3],
            0,
            "corner ({x},{y}) must be transparent"
        );
    }
}

#[test]
fn apply_mask_zeroes_corner_alpha() {
    let mut img = solid(64, 64, [255, 255, 255, 255]);
    apply_mask(&mut img, &circular_mask(64));
    // Center stays opaque, corners become transparent.
    assert_eq!(img.get_pixel(32, 32)[3], 255);
    assert_eq!(img.get_pixel(0, 0)[3], 0);
    assert_eq!(img.get_pixel(63, 63)[3], 0);
}

#[test]
fn image_to_sticker_circle_makes_corners_transparent() {
    let src = solid(600, 600, [120, 200, 80, 255]);
    let bytes = image_to_sticker(
        &to_png(&src),
        FilePreprocessType::Circle,
        &PhotoQuoteArguments::default(),
    )
    .unwrap();
    let out = decode(&bytes);
    assert_eq!(out.dimensions(), (512, 512));
    assert_eq!(out.get_pixel(256, 256)[3], 255);
    assert_eq!(out.get_pixel(0, 0)[3], 0);
}

#[test]
fn image_to_sticker_circle_non_square_source_silhouette_is_round() {
    // Wide source: before the fix, the mask was stretched and the silhouette
    // came out oval. The shape must be a true circle regardless of aspect.
    let src = solid(1000, 400, [120, 200, 80, 255]);
    let bytes = image_to_sticker(
        &to_png(&src),
        FilePreprocessType::Circle,
        &PhotoQuoteArguments::default(),
    )
    .unwrap();
    let out = decode(&bytes);
    assert_eq!(out.dimensions(), (512, 512));
    // Points along the cardinal axes near the radius should all be ~opaque;
    // an oval mask would make top/bottom transparent while leaving left/right opaque.
    // Mask radius is ~249 px; probe at 240 leaves comfortable headroom on all
    // four cardinal axes. An oval mask would have transparent top/bottom here.
    let r = 240;
    let c = 256;
    assert!(out.get_pixel(c, c - r)[3] > 200, "top edge should be inside");
    assert!(out.get_pixel(c, c + r)[3] > 200, "bottom edge should be inside");
    assert!(out.get_pixel(c - r, c)[3] > 200, "left edge should be inside");
    assert!(out.get_pixel(c + r, c)[3] > 200, "right edge should be inside");
    assert_eq!(out.get_pixel(0, 0)[3], 0);
}

#[test]
fn image_to_sticker_default_preserves_color() {
    let src = solid(800, 800, [10, 20, 30, 255]);
    let bytes = image_to_sticker(
        &to_png(&src),
        FilePreprocessType::Default,
        &PhotoQuoteArguments::default(),
    )
    .unwrap();
    let out = decode(&bytes);
    assert_eq!(out.dimensions(), (512, 512));
    let p = out.get_pixel(256, 256);
    assert!((p[0] as i32 - 10).abs() <= 2);
    assert!((p[1] as i32 - 20).abs() <= 2);
    assert!((p[2] as i32 - 30).abs() <= 2);
}

#[test]
fn image_to_sticker_emoji_size_is_100() {
    let src = solid(800, 800, [10, 20, 30, 255]);
    let args = PhotoQuoteArguments {
        is_emoji: Some(true),
        ..Default::default()
    };
    let bytes = image_to_sticker(&to_png(&src), FilePreprocessType::Default, &args).unwrap();
    let out = decode(&bytes);
    assert_eq!(out.dimensions(), (100, 100));
}

#[test]
fn over_opaque_replaces_base() {
    let mut base = solid(4, 4, [0, 0, 0, 255]);
    let top = solid(4, 4, [255, 0, 0, 255]);
    over(&mut base, &top);
    for p in base.pixels() {
        assert_eq!(p, &Rgba([255, 0, 0, 255]));
    }
}

#[test]
fn over_transparent_top_leaves_base() {
    let original = solid(4, 4, [42, 80, 200, 255]);
    let mut base = original.clone();
    let top = solid(4, 4, [0, 0, 0, 0]);
    over(&mut base, &top);
    assert_eq!(base.as_raw(), original.as_raw());
}

#[test]
fn over_half_alpha_blends() {
    // 50% red on top of solid black → roughly (127, 0, 0, 255)
    let mut base = solid(2, 2, [0, 0, 0, 255]);
    let top = solid(2, 2, [255, 0, 0, 128]);
    over(&mut base, &top);
    let p = base.get_pixel(0, 0);
    assert!((p[0] as i32 - 128).abs() <= 2, "got {}", p[0]);
    assert_eq!(p[1], 0);
    assert_eq!(p[2], 0);
    assert_eq!(p[3], 255);
}

#[test]
fn image_to_sticker_with_bubble_overlay_changes_pixels() {
    // Verify the bubble is actually composited — output should differ from
    // the no-bubble version in at least one pixel.
    let src = solid(400, 400, [200, 200, 200, 255]);
    let no_bubble = image_to_sticker(
        &to_png(&src),
        FilePreprocessType::Default,
        &PhotoQuoteArguments::default(),
    )
    .unwrap();
    let with_bubble = image_to_sticker(
        &to_png(&src),
        FilePreprocessType::Default,
        &PhotoQuoteArguments {
            speech_bubble: Some(0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_ne!(no_bubble, with_bubble);
}
