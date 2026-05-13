use image::Rgba;
use mallard_bot::quote::{render_quote, QUOTE_HEIGHT, QUOTE_WIDTH};

fn decode(bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory(bytes).unwrap().to_rgba8()
}

#[test]
fn quote_output_has_expected_dimensions() {
    let bytes = render_quote("hello", "world", 0).unwrap();
    let img = decode(&bytes);
    assert_eq!(img.dimensions(), (QUOTE_WIDTH, QUOTE_HEIGHT));
}

#[test]
fn quote_color_index_picks_palette_color() {
    // Corner pixel should sit on the background (no glyphs reach the edge).
    let yellow_bytes = render_quote("hi", "a", 0).unwrap();
    let green_bytes = render_quote("hi", "a", 1).unwrap();
    let yellow = decode(&yellow_bytes);
    let green = decode(&green_bytes);
    assert_eq!(yellow.get_pixel(0, 0), &Rgba([248, 205, 48, 255]));
    assert_eq!(green.get_pixel(0, 0), &Rgba([75, 151, 75, 255]));
}

#[test]
fn quote_color_index_wraps() {
    // 4 is past the palette length (4 colors); should wrap to index 0.
    let a = decode(&render_quote("hi", "a", 0).unwrap());
    let b = decode(&render_quote("hi", "a", 4).unwrap());
    assert_eq!(a.as_raw(), b.as_raw());
}

#[test]
fn quote_render_is_deterministic() {
    let a = render_quote("Hello, world!", "Author Name", 2).unwrap();
    let b = render_quote("Hello, world!", "Author Name", 2).unwrap();
    assert_eq!(a, b, "render_quote must be deterministic for golden tests");
}

#[test]
fn quote_text_paints_non_background_pixels() {
    let bytes = render_quote("AAAAA AAAAA AAAAA", "Author", 0).unwrap();
    let img = decode(&bytes);
    let bg = Rgba([248, 205, 48, 255]);
    let painted = img.pixels().filter(|p| **p != bg).count();
    assert!(
        painted > 100,
        "expected glyphs to have rendered; painted={painted}"
    );
}
