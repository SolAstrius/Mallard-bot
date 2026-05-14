//! Render a few sample inputs to PNG for manual inspection.
//! `nix-shell -p typst --run 'cargo run --example typst_samples'`

use mallard_bot::typst::{render_png, RenderOpts};

#[tokio::main]
async fn main() {
    let out_dir = "/tmp/typst-samples";
    std::fs::create_dir_all(out_dir).unwrap();

    let cases: &[(&str, &str)] = &[
        ("math_simple", "x^2 + 1"),
        (
            "math_complex",
            "integral_0^infinity e^(-x^2) dif x = sqrt(pi) / 2",
        ),
        ("math_matrix", "mat(1, 2; 3, 4)"),
        ("math_sum", "sum_(k=1)^n k = n(n+1)/2"),
        (
            "with_text",
            "#text(size: 20pt)[The Pythagorean theorem:] $ a^2 + b^2 = c^2 $",
        ),
        (
            "doc",
            "#set page(width: 8cm, height: auto)\n= Hello\nThis is *bold* and _italic_.",
        ),
    ];

    let opts = RenderOpts::default();
    for (name, src) in cases {
        match render_png(src, &opts).await {
            Ok(bytes) => {
                let path = format!("{out_dir}/{name}.png");
                std::fs::write(&path, &bytes).unwrap();
                println!("{name}: {path} ({} bytes)", bytes.len());
            }
            Err(e) => println!("{name}: ERR {e}"),
        }
    }
}
