//! Render Typst snippets to PNG via the `typst` CLI.
//!
//! For chat ergonomics: if the source contains no `#` or `$`, the whole
//! input is wrapped as `$ ... $` so `/typst x^2 + 1` Just Works as math.
//! Otherwise the source is rendered as a full document. A small preamble
//! (auto-sized page, larger text) is prepended either way.

use std::time::Duration;

use tempfile::TempDir;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct RenderOpts {
    pub ppi: u32,
    pub text_size_pt: f32,
    pub timeout: Duration,
    pub max_input_bytes: usize,
    /// Where `@preview/...` packages get cached. `None` means the typst
    /// CLI's default (`$XDG_CACHE_HOME/typst/packages`).
    pub package_cache_path: Option<std::path::PathBuf>,
    /// If the rendered PNG is smaller than this in either dimension, it's
    /// padded with a white background to bring it up to baseline. Telegram
    /// previews tiny images as postage stamps; this keeps `/typst x^2 + 1`
    /// readable in chat.
    pub min_width_px: u32,
    pub min_height_px: u32,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            ppi: 300,
            text_size_pt: 16.0,
            // Typst rescans system fonts on each invocation — even a tiny
            // formula sits at 2–3 s. 15 s gives headroom for cold runs +
            // mildly complex docs without letting infinite-loop renders
            // hang the bot.
            timeout: Duration::from_secs(15),
            max_input_bytes: 16 * 1024,
            package_cache_path: None,
            min_width_px: 800,
            min_height_px: 400,
        }
    }
}

#[derive(Debug)]
pub enum RenderError {
    TooLong { len: usize, max: usize },
    Timeout,
    Compile(String),
    Spawn(std::io::Error),
    Io(std::io::Error),
    NoOutput,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong { len, max } => {
                write!(f, "input too long ({len} > {max} bytes)")
            }
            Self::Timeout => write!(f, "compile timed out"),
            Self::Compile(s) => write!(f, "compile error:\n{s}"),
            Self::Spawn(e) => write!(f, "spawn typst: {e}"),
            Self::Io(e) => write!(f, "i/o: {e}"),
            Self::NoOutput => write!(f, "typst produced no output"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Build the actual `.typ` document fed to the compiler. Pure function —
/// no I/O, safe to unit-test.
pub fn assemble(src: &str, opts: &RenderOpts) -> String {
    let trimmed = src.trim();
    let looks_like_doc = trimmed.contains('#') || trimmed.contains('$');
    let body = if looks_like_doc {
        trimmed.to_string()
    } else {
        format!("$ {trimmed} $")
    };
    format!(
        "#set page(width: auto, height: auto, margin: (x: 10pt, y: 8pt))\n\
         #set text(size: {size}pt)\n\
         {body}\n",
        size = opts.text_size_pt,
        body = body,
    )
}

pub async fn render_png(source: &str, opts: &RenderOpts) -> Result<Vec<u8>, RenderError> {
    if source.len() > opts.max_input_bytes {
        return Err(RenderError::TooLong {
            len: source.len(),
            max: opts.max_input_bytes,
        });
    }

    let doc = assemble(source, opts);
    let dir = TempDir::new().map_err(RenderError::Io)?;
    let input_path = dir.path().join("input.typ");
    let output_path = dir.path().join("output.png");
    tokio::fs::write(&input_path, &doc)
        .await
        .map_err(RenderError::Io)?;

    let mut cmd = Command::new("typst");
    cmd.arg("compile")
        .args(["--format", "png"])
        .args(["--ppi", &opts.ppi.to_string()])
        .arg("--root")
        .arg(dir.path())
        .arg(&input_path)
        .arg(&output_path)
        .kill_on_drop(true);

    if let Some(cache) = &opts.package_cache_path {
        cmd.arg("--package-cache-path").arg(cache);
    }

    let output_res = match tokio::time::timeout(opts.timeout, cmd.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(RenderError::Spawn(e)),
        Err(_) => return Err(RenderError::Timeout),
    };

    if !output_res.status.success() {
        let stderr = String::from_utf8_lossy(&output_res.stderr).into_owned();
        return Err(RenderError::Compile(stderr.trim().to_string()));
    }

    if !output_path.exists() {
        return Err(RenderError::NoOutput);
    }
    let bytes = tokio::fs::read(&output_path)
        .await
        .map_err(RenderError::Io)?;
    Ok(pad_to_minimum(
        &bytes,
        opts.min_width_px,
        opts.min_height_px,
    )
    .unwrap_or(bytes))
}

/// Center the PNG on a white canvas of at least `min_w × min_h` pixels.
/// Returns `None` (caller falls back to the original bytes) if decoding or
/// re-encoding fails for any reason — padding is best-effort.
fn pad_to_minimum(bytes: &[u8], min_w: u32, min_h: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    if w >= min_w && h >= min_h {
        return None;
    }
    let out_w = w.max(min_w);
    let out_h = h.max(min_h);
    let mut canvas =
        image::RgbaImage::from_pixel(out_w, out_h, image::Rgba([255, 255, 255, 255]));
    let dx = ((out_w - w) / 2) as i64;
    let dy = ((out_h - h) / 2) as i64;
    image::imageops::overlay(&mut canvas, &img.to_rgba8(), dx, dy);
    let mut out = Vec::with_capacity(bytes.len() + 1024);
    image::DynamicImage::ImageRgba8(canvas)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assemble_wraps_bare_math() {
        let out = assemble("x^2 + 1", &RenderOpts::default());
        assert!(out.contains("$ x^2 + 1 $"), "got: {out}");
    }

    #[test]
    fn assemble_leaves_document_alone() {
        let src = "#set page(width: 5cm)\nhello";
        let out = assemble(src, &RenderOpts::default());
        assert!(out.contains("#set page(width: 5cm)"));
        // No extra $ wrapping
        assert!(!out.contains("$ #set"));
    }

    #[test]
    fn assemble_leaves_explicit_math_alone() {
        let src = "result is $ x^2 $";
        let out = assemble(src, &RenderOpts::default());
        assert!(out.ends_with("result is $ x^2 $\n"), "got: {out}");
    }

    /// Real render — requires `typst` on PATH. Skipped if it isn't.
    #[tokio::test]
    async fn render_simple_math_when_typst_present() {
        let exists = Command::new("typst")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !exists {
            eprintln!("skipping render test — typst not on PATH");
            return;
        }
        let bytes = render_png("x^2 + 1", &RenderOpts::default())
            .await
            .expect("render");
        // PNG magic number
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        assert!(bytes.len() > 200, "png too small: {} bytes", bytes.len());
    }

    #[tokio::test]
    async fn render_reports_compile_errors() {
        let exists = Command::new("typst")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !exists {
            return;
        }
        // Use a hash to skip math-wrapping, then trigger a real error.
        let err = render_png("#bogus_function_name()", &RenderOpts::default())
            .await
            .expect_err("should fail");
        match err {
            RenderError::Compile(_) => {}
            other => panic!("expected Compile, got: {other}"),
        }
    }

    #[tokio::test]
    async fn render_rejects_oversized_input() {
        let big = "a".repeat(20_000);
        let opts = RenderOpts {
            max_input_bytes: 16 * 1024,
            ..Default::default()
        };
        let err = render_png(&big, &opts).await.expect_err("should reject");
        assert!(matches!(err, RenderError::TooLong { .. }));
    }
}
