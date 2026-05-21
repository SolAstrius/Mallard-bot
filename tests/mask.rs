//! Sanity coverage for procedural mask shapes.

use mallard_bot::mask::{render, Mask};

#[test]
fn from_preset_round_trip() {
    for (name, expected) in [
        ("circle", Mask::Circle),
        ("square", Mask::Square),
        ("triangle", Mask::Triangle),
        ("diamond", Mask::Diamond),
        ("hexagon", Mask::Hexagon),
        ("star", Mask::Star),
    ] {
        assert_eq!(Mask::from_preset(name), Some(expected));
    }
    assert_eq!(Mask::from_preset("hexagram"), None);
    assert_eq!(Mask::from_preset("Circle"), None);
}

#[test]
fn render_circle_center_opaque_corners_clear() {
    let m = render(Mask::Circle, 128);
    assert_eq!(m.get_pixel(64, 64)[3], 255);
    assert_eq!(m.get_pixel(0, 0)[3], 0);
    assert_eq!(m.get_pixel(127, 127)[3], 0);
}

#[test]
fn render_square_corners_opaque_circle_corners_clear() {
    // The square mask reaches into the corners (with a small AA-padding inset);
    // the circle does not. This is the cheapest discriminator between shapes.
    let sq = render(Mask::Square, 128);
    let ci = render(Mask::Circle, 128);
    // ~6% inset matches the 0.976 scale factor; pixel (8,8) is well inside the
    // square but far outside the inscribed circle.
    assert!(sq.get_pixel(8, 8)[3] > 200, "square should cover near-corner");
    assert_eq!(ci.get_pixel(8, 8)[3], 0, "circle should not");
}

#[test]
fn render_diamond_axes_opaque_corners_clear() {
    let m = render(Mask::Diamond, 128);
    let cx = 64;
    assert!(m.get_pixel(cx, cx)[3] > 200, "center opaque");
    assert!(m.get_pixel(cx, 8)[3] > 200, "top tip inside");
    assert!(m.get_pixel(8, cx)[3] > 200, "left tip inside");
    assert_eq!(m.get_pixel(0, 0)[3], 0, "corner clear");
    assert_eq!(m.get_pixel(127, 0)[3], 0);
}

#[test]
fn render_triangle_apex_up_base_down() {
    let m = render(Mask::Triangle, 128);
    // Apex near top-center, wide base near bottom. Top-center should be opaque;
    // bottom-center opaque; top-left/right corners clear.
    assert!(m.get_pixel(64, 6)[3] > 200, "apex");
    // The base sits at y = -0.5 in normalized coords (inscribed in unit circle),
    // which lands around pixel y ≈ 95 for a 128px render — not the image edge.
    assert!(m.get_pixel(64, 90)[3] > 200, "base center");
    assert_eq!(m.get_pixel(64, 124)[3], 0, "below base is clear");
    assert_eq!(m.get_pixel(0, 0)[3], 0);
    // Top-left and top-right should be clear (triangle narrows toward apex).
    assert_eq!(m.get_pixel(10, 10)[3], 0);
}

#[test]
fn render_hexagon_center_and_apex() {
    let m = render(Mask::Hexagon, 128);
    assert!(m.get_pixel(64, 64)[3] > 200);
    // Pointy-top: top and bottom apexes should be (nearly) inside.
    assert!(m.get_pixel(64, 6)[3] > 200);
    assert!(m.get_pixel(64, 120)[3] > 200);
    // Corners are outside.
    assert_eq!(m.get_pixel(0, 0)[3], 0);
}

#[test]
fn render_star_center_opaque_concave_gaps() {
    let m = render(Mask::Star, 128);
    assert!(m.get_pixel(64, 64)[3] > 200);
    // A star has concave notches between points: a midline angle between two
    // outer points sits in a gap. The far-right midline (y = center, x ~ near
    // right edge) lies in one such notch.
    assert_eq!(m.get_pixel(120, 64)[3], 0, "concave gap on the right side");
    assert_eq!(m.get_pixel(0, 0)[3], 0);
}
