//! Render a few sample plot inputs to PNG for manual inspection.
//! `nix-shell -p typst m4 gmp mpfr libmpc pkg-config --run 'cargo run --example plot_samples'`

use mallard_bot::plot;
use mallard_bot::typst::RenderOpts;

#[tokio::main]
async fn main() {
    let out_dir = "/tmp/plot-samples";
    std::fs::create_dir_all(out_dir).unwrap();

    let cases: &[(&str, &str)] = &[
        ("sin", "plot(sin(x), 0, 2*pi)"),
        ("sin_cos", "plot(sin(x), cos(x), -pi, pi)"),
        ("parabola", "plot(x^2, -3, 3)"),
        ("gaussian", "plot(exp(-x^2), -3, 3)"),
        ("damped", "plot(exp(-x/3) * cos(x), 0, 10)"),
        ("sigmoid_logs", "plot(1/(1+exp(-x)), ln(1+x^2), -5, 5)"),
    ];

    let opts = RenderOpts::default();
    for (name, src) in cases {
        match plot::render(src, &opts).await {
            Ok(pages) => {
                for (i, bytes) in pages.iter().enumerate() {
                    let suffix = if pages.len() == 1 {
                        String::new()
                    } else {
                        format!("-{}", i + 1)
                    };
                    let path = format!("{out_dir}/{name}{suffix}.png");
                    std::fs::write(&path, bytes).unwrap();
                    println!("{name}{suffix}: {path} ({} bytes)", bytes.len());
                }
            }
            Err(e) => println!("{name}: ERR {e}"),
        }
    }
}
