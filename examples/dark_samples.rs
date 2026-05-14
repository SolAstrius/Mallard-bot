//! Render math + plot samples in dark theme for visual check.
//! `nix-shell -p m4 gmp mpfr libmpc pkg-config typst --run 'cargo run --example dark_samples'`

use mallard_bot::math::{render as math_render, Dialect};
use mallard_bot::plot;
use mallard_bot::typst::{RenderOpts, Theme};

#[tokio::main]
async fn main() {
    std::fs::create_dir_all("/tmp/dark-samples").unwrap();
    let opts = RenderOpts {
        theme: Theme::Dark,
        ..RenderOpts::default()
    };

    let math_cases: &[(&str, Dialect)] = &[
        ("x^2 + 1", Dialect::Typst),
        ("\\frac{1}{2} + \\int_0^\\infty e^{-x^2} dx", Dialect::Latex),
        ("mat(1, 2; 3, 4)", Dialect::Typst),
    ];
    for (i, (src, dialect)) in math_cases.iter().enumerate() {
        let pages = math_render(src, *dialect, &opts).await.unwrap();
        let path = format!("/tmp/dark-samples/math_{i}.png");
        std::fs::write(&path, &pages[0]).unwrap();
        println!("math_{i}: {path}");
    }

    let plot_cases: &[(&str, &str)] = &[
        ("sin", "plot(sin(x), 0, 2*pi)"),
        ("sin_cos", "plot(sin(x), cos(x), -pi, pi)"),
        ("gaussian", "plot(exp(-x^2), -3, 3)"),
    ];
    for (name, src) in plot_cases {
        let pages = plot::render(src, &opts).await.unwrap();
        let path = format!("/tmp/dark-samples/plot_{name}.png");
        std::fs::write(&path, &pages[0]).unwrap();
        println!("plot_{name}: {path}");
    }
}
