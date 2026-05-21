//! Video sticker / emoji pipeline.
//!
//! Pure-Rust VP9/WebM encoding is not production-ready (ffmpeg-next bindings
//! exist but pull libavcodec at build time, undermining the "native" goal),
//! so this module deliberately shells out to `ffmpeg`. The Rust side owns the
//! filter graph composition, sizing math, and error mapping that lived in
//! stickers.py — only the codec work is delegated.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use image::ImageEncoder;
use tempfile::NamedTempFile;
use tokio::fs;
use tokio::process::Command;

use crate::arguments::VideoQuoteArguments;
use crate::exceptions::{ProcessingError, ProcessingErrorKind};
use crate::imaging::{desired_size, load_bubble};
use crate::mask::{self, Mask};

const FFMPEG_TIMEOUT: Duration = Duration::from_secs(75);
const FFMPEG_BIN: &str = "ffmpeg";
/// Telegram's hard cap on a video sticker payload.
const STICKER_BYTES_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoPreprocess {
    Default,
    Mask(Mask),
    VideoThumb,
}

fn err_args(msg: impl Into<String>) -> ProcessingError {
    ProcessingError::new(ProcessingErrorKind::ArgumentsParsingError, msg)
}

fn err_unexpected(msg: impl Into<String>) -> ProcessingError {
    ProcessingError::new(ProcessingErrorKind::Unexpected, msg)
}

/// Detect (width, height, duration_seconds) of a video file via ffprobe-like
/// invocation of ffmpeg (we use a single ffprobe shell-out for simplicity).
async fn probe(path: &Path) -> Result<(u32, u32, f64), ProcessingError> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,duration",
            "-of",
            "default=noprint_wrappers=1:nokey=0",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| err_unexpected(format!("ffprobe spawn: {e}")))?;
    if !output.status.success() {
        return Err(err_unexpected(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut w = 0u32;
    let mut h = 0u32;
    let mut dur = 0.0f64;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let (k, v) = line.split_once('=').unwrap_or(("", ""));
        match k {
            "width" => w = v.parse().unwrap_or(0),
            "height" => h = v.parse().unwrap_or(0),
            "duration" => dur = v.parse().unwrap_or(0.0),
            _ => {}
        }
    }
    Ok((w, h, dur))
}

/// Resolve the `s`/`e`/`x`/`r` interactions exactly as stickers.py did, so
/// arguments produce the same trim/speed/reverse semantics.
pub fn resolve_arguments(
    raw: VideoQuoteArguments,
    duration_seconds: f64,
) -> Result<ResolvedVideoArgs, ProcessingError> {
    let mut a = raw;
    if a.end_point.is_some() && a.starting_point.is_some() && a.end_point <= a.starting_point {
        return Err(err_args(
            "Секунда завершения должна быть строго больше секунды начала.",
        ));
    }
    let speed = a.speed.unwrap_or(1.0);
    let reverse = a.reverse.unwrap_or(false);

    let length = duration_seconds.round() as i64;

    let start = match a.starting_point.take() {
        None => 0,
        Some(s) if s > length => {
            return Err(err_args(
                "Секунда начала должна быть меньше чем длина видео.",
            ));
        }
        Some(s) => {
            let s = if reverse { length - s } else { s };
            ((s as f64) * (1.0 / speed)).round() as i64
        }
    };

    let mut final_length = a.final_length.unwrap_or(2.9);
    let end = a.end_point.take().map(|e| {
        let e = if reverse { length - e } else { e };
        ((e as f64) * (1.0 / speed)).round() as i64
    });

    let (start, end) = if let Some(end) = end {
        let (s, e) = if reverse { (end, start) } else { (start, end) };
        final_length = (e - s).unsigned_abs() as f64;
        (s, Some(e))
    } else {
        (start, end)
    };

    final_length = final_length.clamp(0.1, 2.9);

    Ok(ResolvedVideoArgs {
        starting_point: start.max(0),
        end_point: end,
        speed,
        reverse,
        final_length,
        speech_bubble: a.speech_bubble,
        is_emoji: a.is_emoji.unwrap_or(false),
    })
}

#[derive(Debug, Clone)]
pub struct ResolvedVideoArgs {
    pub starting_point: i64,
    pub end_point: Option<i64>,
    pub speed: f64,
    pub reverse: bool,
    pub final_length: f64,
    pub speech_bubble: Option<usize>,
    pub is_emoji: bool,
}

fn fit(w: u32, h: u32, desired: u32) -> (u32, u32) {
    if w >= h {
        (
            desired,
            ((h as f64) * (desired as f64 / w as f64)).round() as u32,
        )
    } else {
        (
            ((w as f64) * (desired as f64 / h as f64)).round() as u32,
            desired,
        )
    }
}

/// Optional `(side, x, y, target)` describing a center-square crop of the
/// source followed by a scale to `target × target`. Required for `Mask(_)`.
fn build_filter_graph(
    args: &ResolvedVideoArgs,
    preprocess: VideoPreprocess,
    square_crop: Option<(u32, u32, u32, u32)>,
) -> String {
    let pts = format!("setpts={:.2}*PTS", 1.0 / args.speed);
    match preprocess {
        VideoPreprocess::Mask(_) => {
            let (side, x, y, target) =
                square_crop.expect("masked preprocess requires square_crop");
            let mut g = format!(
                "[0:v]crop={side}:{side}:{x}:{y},scale={target}:{target}[sq];\
                 [1:v]alphaextract[alf];[sq][alf]alphamerge[res];"
            );
            if args.speech_bubble.is_some() {
                g.push_str("[res][2:v]overlay[res];");
            }
            if args.reverse {
                g.push_str("[res]reverse[res];");
            }
            g.push_str(&format!("[res]{pts}"));
            g
        }
        _ => {
            let mut g = format!("[0:v]{pts}[res];");
            if args.speech_bubble.is_some() {
                g.push_str("[res][1:v]overlay[res];");
            }
            if args.reverse {
                g.push_str("[res]reverse[res];");
            }
            // strip trailing tag if present
            if g.ends_with("[res];") {
                g.truncate(g.len() - "[res];".len());
            }
            g
        }
    }
}

async fn write_png(path: &Path, img: &image::RgbaImage) -> Result<(), ProcessingError> {
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| err_unexpected(format!("encode mask: {e}")))?;
    fs::write(path, buf)
        .await
        .map_err(|e| err_unexpected(format!("write mask: {e}")))
}

/// Format an integer seconds offset as `00:00:NN` (zero-padded).
fn fmt_ts(secs: i64) -> String {
    let secs = secs.max(0);
    format!("00:00:{:02}", secs)
}

/// Run the full pipeline: takes video bytes, produces WebM bytes.
pub async fn video_to_sticker(
    bytes: &[u8],
    args: VideoQuoteArguments,
    preprocess: VideoPreprocess,
) -> Result<Vec<u8>, ProcessingError> {
    let input =
        NamedTempFile::with_suffix(".mp4").map_err(|e| err_unexpected(format!("tempfile: {e}")))?;
    fs::write(input.path(), bytes)
        .await
        .map_err(|e| err_unexpected(format!("write input: {e}")))?;

    let (w, h, duration) = probe(input.path()).await?;
    if w == 0 || h == 0 {
        return Err(err_unexpected("could not determine video dimensions"));
    }
    let resolved = resolve_arguments(args, duration)?;
    let target = desired_size(resolved.is_emoji);
    let masked = matches!(preprocess, VideoPreprocess::Mask(_));
    let (new_w, new_h) = if masked {
        (target, target)
    } else {
        fit(w, h, target)
    };
    let square_crop = if masked {
        let side = w.min(h);
        Some((side, (w - side) / 2, (h - side) / 2, target))
    } else {
        None
    };

    let output = NamedTempFile::with_suffix(".webm")
        .map_err(|e| err_unexpected(format!("tempfile: {e}")))?;

    let mask_file;
    let bubble_file;

    let mut cmd = Command::new(FFMPEG_BIN);
    cmd.args(["-nostats", "-loglevel", "error", "-y"])
        .arg("-i")
        .arg(input.path());

    if let VideoPreprocess::Mask(m) = preprocess {
        mask_file = NamedTempFile::with_suffix(".png")
            .map_err(|e| err_unexpected(format!("mask tempfile: {e}")))?;
        let mask_img = mask::render(m, target);
        write_png(mask_file.path(), &mask_img).await?;
        cmd.arg("-loop").arg("1").arg("-i").arg(mask_file.path());
    }

    if let Some(idx) = resolved.speech_bubble {
        let (bw, bh) = if masked { (target, target) } else { (w, h) };
        let bubble = load_bubble(idx, bw, bh)?;
        bubble_file = NamedTempFile::with_suffix(".png")
            .map_err(|e| err_unexpected(format!("bubble tempfile: {e}")))?;
        write_png(bubble_file.path(), &bubble).await?;
        cmd.arg("-i").arg(bubble_file.path());
    }

    cmd.arg("-filter_complex")
        .arg(build_filter_graph(&resolved, preprocess, square_crop))
        // VP9 with alpha for masked output, plain VP9 otherwise.
        .args(["-c:v", "libvpx-vp9", "-auto-alt-ref", "0"]);
    if masked {
        cmd.args(["-pix_fmt", "yuva420p"]);
    } else {
        cmd.args(["-pix_fmt", "yuv420p"]);
    }
    cmd.args(["-deadline", "good", "-cpu-used", "4", "-row-mt", "1"])
        .args(["-ss", &fmt_ts(resolved.starting_point)]);
    if let Some(end) = resolved.end_point {
        cmd.args(["-to", &fmt_ts(end)]);
    }
    cmd.args(["-s", &format!("{new_w}x{new_h}")])
        .args(["-t", &format!("{:.2}", resolved.final_length)])
        .arg("-an");

    // Two-pass strategy: first encode aims for ≈250KB at the configured CRF;
    // if it overshoots Telegram's 256KB cap we re-run with a tighter cap.
    let attempts: &[(&str, &str, &str)] = &[
        // (CRF, max bitrate, buffer)
        ("32", "550k", "1M"),
        ("38", "300k", "600k"),
        ("44", "180k", "360k"),
    ];

    for (i, (crf, maxrate, bufsize)) in attempts.iter().enumerate() {
        let mut attempt = clone_cmd(&cmd);
        // libvpx-vp9 "constrained quality" mode: -b:v must equal the cap when
        // -maxrate/-bufsize are present (using -b:v 0 here yields a "rate
        // control parameters set without a bitrate" error from the encoder).
        attempt
            .args(["-crf", crf, "-b:v", maxrate])
            .args(["-maxrate", maxrate, "-bufsize", bufsize])
            .arg("-y")
            .arg(output.path())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let child = attempt
            .spawn()
            .map_err(|e| err_unexpected(format!("spawn ffmpeg: {e}")))?;
        let result = tokio::time::timeout(FFMPEG_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| err_unexpected("ffmpeg timeout"))?
            .map_err(|e| err_unexpected(format!("ffmpeg wait: {e}")))?;
        if !result.status.success() {
            return Err(err_unexpected(format!(
                "ffmpeg: {}",
                String::from_utf8_lossy(&result.stderr)
            )));
        }

        let bytes = fs::read(output.path())
            .await
            .map_err(|e| err_unexpected(format!("read output: {e}")))?;
        log::info!(
            "encode attempt {} (crf={crf} maxrate={maxrate}): {} bytes",
            i + 1,
            bytes.len()
        );
        if bytes.len() <= STICKER_BYTES_LIMIT || i == attempts.len() - 1 {
            return Ok(bytes);
        }
        log::warn!("output exceeded {STICKER_BYTES_LIMIT}B, retrying with tighter encode");
    }
    unreachable!()
}

fn clone_cmd(src: &Command) -> Command {
    let mut new = Command::new(src.as_std().get_program());
    for arg in src.as_std().get_args() {
        new.arg(arg);
    }
    if let Some(dir) = src.as_std().get_current_dir() {
        new.current_dir(dir);
    }
    new
}

/// Port of `video2emoji` — minimal pass that re-encodes to a 100×100 WebM.
pub async fn video_to_emoji(bytes: &[u8]) -> Result<Vec<u8>, ProcessingError> {
    let input =
        NamedTempFile::with_suffix(".mp4").map_err(|e| err_unexpected(format!("tempfile: {e}")))?;
    fs::write(input.path(), bytes)
        .await
        .map_err(|e| err_unexpected(format!("write: {e}")))?;
    let output = NamedTempFile::with_suffix(".webm")
        .map_err(|e| err_unexpected(format!("tempfile: {e}")))?;

    let status = Command::new(FFMPEG_BIN)
        .args(["-nostats", "-loglevel", "error", "-y"])
        .arg("-i")
        .arg(input.path())
        .args(["-loop", "1"])
        .args(["-c:v", "libvpx-vp9", "-preset", "ultrafast"])
        .args(["-s", "100x100", "-t", "2.9", "-an"])
        .arg(output.path())
        .status()
        .await
        .map_err(|e| err_unexpected(format!("ffmpeg: {e}")))?;
    if !status.success() {
        return Err(err_unexpected("ffmpeg failed"));
    }
    fs::read(output.path())
        .await
        .map_err(|e| err_unexpected(format!("read: {e}")))
}
