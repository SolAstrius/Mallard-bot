//! Render Typst snippets to PNG via the `typst` CLI.
//!
//! For chat ergonomics: if the source contains no `#` or `$`, the whole
//! input is wrapped as `$ ... $` so `/typst x^2 + 1` Just Works as math.
//! Otherwise the source is rendered as a full document. A small preamble
//! (auto-sized page, larger text) is prepended either way.
//!
//! Multi-page output is returned page-by-page; callers decide whether to
//! send a single photo or an album. A process-wide semaphore caps how many
//! `typst` subprocesses can run concurrently — typst is CPU- and
//! memory-heavy and the bot has neighbours on the same node.

use std::sync::OnceLock;
use std::time::Duration;

use tempfile::TempDir;
use tokio::process::Command;
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
pub struct RenderOpts {
    pub ppi: u32,
    pub text_size_pt: f32,
    pub timeout: Duration,
    pub max_input_bytes: usize,
    /// Where `@preview/...` packages get cached. `None` means the typst
    /// CLI's default (`$XDG_CACHE_HOME/typst/packages`).
    pub package_cache_path: Option<std::path::PathBuf>,
    /// If a rendered page is smaller than this in either dimension, it's
    /// padded with a white background to bring it up to baseline. Telegram
    /// previews tiny images as postage stamps; this keeps `/typst x^2 + 1`
    /// readable in chat.
    pub min_width_px: u32,
    pub min_height_px: u32,
    /// Per-page hard caps. Telegram refuses photos larger than 10 MB or
    /// past its dimension limits anyway — fail fast on our side.
    pub max_dim_px: u32,
    pub max_bytes_per_page: usize,
    /// Telegram media groups accept up to 10 items. Renders past that get
    /// rejected with `TooManyPages` rather than silently truncated.
    pub max_pages: usize,
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
            max_dim_px: 4096,
            max_bytes_per_page: 8 * 1024 * 1024,
            max_pages: 10,
        }
    }
}

#[derive(Debug)]
pub enum RenderError {
    TooLong {
        len: usize,
        max: usize,
    },
    TooLargeDimensions {
        width: u32,
        height: u32,
        max: u32,
    },
    TooLargeBytes {
        bytes: usize,
        max: usize,
    },
    TooManyPages {
        count: usize,
        max: usize,
    },
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
            Self::TooLargeDimensions { width, height, max } => {
                write!(f, "page too big ({width}×{height}, max {max}px per side)")
            }
            Self::TooLargeBytes { bytes, max } => {
                write!(f, "page too heavy ({bytes} > {max} bytes)")
            }
            Self::TooManyPages { count, max } => {
                write!(f, "too many pages ({count}, max {max})")
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

/// Process-wide semaphore around `typst` invocations. Two permits keeps the
/// bot responsive when several chats fire renders simultaneously without
/// pegging the node — typst peaks around 200 MB and one CPU per compile.
fn render_semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(2))
}

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

/// Compile `source` and return one PNG byte-vector per page. Caller chooses
/// `send_photo` vs `send_media_group` based on `.len()`.
pub async fn render(source: &str, opts: &RenderOpts) -> Result<Vec<Vec<u8>>, RenderError> {
    if source.len() > opts.max_input_bytes {
        return Err(RenderError::TooLong {
            len: source.len(),
            max: opts.max_input_bytes,
        });
    }

    let _permit = render_semaphore()
        .acquire()
        .await
        .expect("render semaphore poisoned");

    let doc = assemble(source, opts);
    let dir = TempDir::new().map_err(RenderError::Io)?;
    let input_path = dir.path().join("input.typ");
    // `{p}` is typst's page-number placeholder. With it, single-page and
    // multi-page output share one code path (always `output-1.png` for
    // page 1, `output-2.png` for page 2, …).
    let output_pattern = dir.path().join("output-{p}.png");
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
        .arg(&output_pattern)
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

    // Collect output-N.png in numeric order.
    let mut entries: Vec<(u32, std::path::PathBuf)> = std::fs::read_dir(dir.path())
        .map_err(RenderError::Io)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let stem = name.strip_prefix("output-")?.strip_suffix(".png")?;
            let n: u32 = stem.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    entries.sort_by_key(|(n, _)| *n);

    if entries.is_empty() {
        return Err(RenderError::NoOutput);
    }
    if entries.len() > opts.max_pages {
        return Err(RenderError::TooManyPages {
            count: entries.len(),
            max: opts.max_pages,
        });
    }

    let mut pages = Vec::with_capacity(entries.len());
    for (_, path) in entries {
        let raw = tokio::fs::read(&path).await.map_err(RenderError::Io)?;
        pages.push(process_page(raw, opts)?);
    }
    Ok(pages)
}

/// Decode → enforce dimension cap → pad to baseline if too small → re-encode
/// (only when padding actually happened). The image crate is already a dep
/// of the bot for sticker work.
fn process_page(bytes: Vec<u8>, opts: &RenderOpts) -> Result<Vec<u8>, RenderError> {
    let img = image::load_from_memory(&bytes)
        .map_err(|e| RenderError::Io(std::io::Error::other(e.to_string())))?;
    let (w, h) = (img.width(), img.height());
    if w > opts.max_dim_px || h > opts.max_dim_px {
        return Err(RenderError::TooLargeDimensions {
            width: w,
            height: h,
            max: opts.max_dim_px,
        });
    }

    let needs_pad = w < opts.min_width_px || h < opts.min_height_px;
    let out_bytes = if needs_pad {
        let out_w = w.max(opts.min_width_px);
        let out_h = h.max(opts.min_height_px);
        let mut canvas =
            image::RgbaImage::from_pixel(out_w, out_h, image::Rgba([255, 255, 255, 255]));
        let dx = ((out_w - w) / 2) as i64;
        let dy = ((out_h - h) / 2) as i64;
        image::imageops::overlay(&mut canvas, &img.to_rgba8(), dx, dy);
        let mut out = Vec::with_capacity(bytes.len() + 1024);
        image::DynamicImage::ImageRgba8(canvas)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .map_err(|e| RenderError::Io(std::io::Error::other(e.to_string())))?;
        out
    } else {
        bytes
    };

    if out_bytes.len() > opts.max_bytes_per_page {
        return Err(RenderError::TooLargeBytes {
            bytes: out_bytes.len(),
            max: opts.max_bytes_per_page,
        });
    }
    Ok(out_bytes)
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
        assert!(!out.contains("$ #set"));
    }

    #[test]
    fn assemble_leaves_explicit_math_alone() {
        let src = "result is $ x^2 $";
        let out = assemble(src, &RenderOpts::default());
        assert!(out.ends_with("result is $ x^2 $\n"), "got: {out}");
    }

    async fn typst_on_path() -> bool {
        Command::new("typst")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn render_simple_math_when_typst_present() {
        if !typst_on_path().await {
            eprintln!("skipping render test — typst not on PATH");
            return;
        }
        let pages = render("x^2 + 1", &RenderOpts::default())
            .await
            .expect("render");
        assert_eq!(pages.len(), 1);
        assert_eq!(&pages[0][0..8], b"\x89PNG\r\n\x1a\n");
        assert!(pages[0].len() > 200);
    }

    #[tokio::test]
    async fn render_multi_page_returns_each_page() {
        if !typst_on_path().await {
            return;
        }
        let src = "page one\n#pagebreak()\npage two\n#pagebreak()\npage three";
        let pages = render(src, &RenderOpts::default()).await.expect("render");
        assert_eq!(pages.len(), 3);
        for p in &pages {
            assert_eq!(&p[0..8], b"\x89PNG\r\n\x1a\n");
        }
    }

    #[tokio::test]
    async fn render_rejects_too_many_pages() {
        if !typst_on_path().await {
            return;
        }
        let src = "1\n#pagebreak()\n2\n#pagebreak()\n3\n#pagebreak()\n4";
        let opts = RenderOpts {
            max_pages: 2,
            ..Default::default()
        };
        let err = render(src, &opts).await.expect_err("should fail");
        assert!(matches!(err, RenderError::TooManyPages { .. }));
    }

    #[tokio::test]
    async fn render_reports_compile_errors() {
        if !typst_on_path().await {
            return;
        }
        let err = render("#bogus_function_name()", &RenderOpts::default())
            .await
            .expect_err("should fail");
        assert!(matches!(err, RenderError::Compile(_)), "got: {err:?}");
    }

    #[tokio::test]
    async fn render_rejects_oversized_input() {
        let big = "a".repeat(20_000);
        let opts = RenderOpts {
            max_input_bytes: 16 * 1024,
            ..Default::default()
        };
        let err = render(&big, &opts).await.expect_err("should reject");
        assert!(matches!(err, RenderError::TooLong { .. }));
    }
}
