//! Tests for the pure-Rust portion of the video pipeline (argument
//! resolution + filter graph composition). The ffmpeg-driven end-to-end path
//! is exercised only when an `ffmpeg` binary is on PATH; otherwise the test
//! is skipped so CI without ffmpeg installed remains green.

use mallard_bot::arguments::VideoQuoteArguments;
use mallard_bot::video::{resolve_arguments, video_to_emoji, video_to_sticker, VideoPreprocess};

#[test]
fn resolve_defaults() {
    let r = resolve_arguments(VideoQuoteArguments::default(), 10.0).unwrap();
    assert_eq!(r.starting_point, 0);
    assert_eq!(r.end_point, None);
    assert_eq!(r.speed, 1.0);
    assert!(!r.reverse);
    assert_eq!(r.final_length, 2.9);
    assert!(!r.is_emoji);
}

#[test]
fn resolve_speed_scales_endpoints() {
    let r = resolve_arguments(
        VideoQuoteArguments {
            starting_point: Some(2),
            end_point: Some(6),
            speed: Some(2.0),
            ..Default::default()
        },
        10.0,
    )
    .unwrap();
    assert_eq!(r.starting_point, 1);
    assert_eq!(r.end_point, Some(3));
    assert_eq!(r.final_length, 2.0);
}

#[test]
fn resolve_reverse_flips_endpoints() {
    let r = resolve_arguments(
        VideoQuoteArguments {
            starting_point: Some(2),
            end_point: Some(5),
            reverse: Some(true),
            ..Default::default()
        },
        10.0,
    )
    .unwrap();
    // reverse swaps start/end after re-anchoring against duration.
    assert!(r.starting_point < r.end_point.unwrap());
    assert!(r.reverse);
}

#[test]
fn resolve_rejects_start_after_video_end() {
    let err = resolve_arguments(
        VideoQuoteArguments {
            starting_point: Some(99),
            ..Default::default()
        },
        10.0,
    )
    .unwrap_err();
    assert_eq!(
        err.kind,
        mallard_bot::ProcessingErrorKind::ArgumentsParsingError
    );
}

#[test]
fn resolve_rejects_end_le_start() {
    let err = resolve_arguments(
        VideoQuoteArguments {
            starting_point: Some(5),
            end_point: Some(5),
            ..Default::default()
        },
        10.0,
    )
    .unwrap_err();
    assert_eq!(
        err.kind,
        mallard_bot::ProcessingErrorKind::ArgumentsParsingError
    );
}

#[test]
fn resolve_final_length_clamped() {
    let r = resolve_arguments(
        VideoQuoteArguments {
            starting_point: Some(0),
            end_point: Some(100),
            ..Default::default()
        },
        200.0,
    )
    .unwrap();
    assert!(r.final_length <= 2.9);
    assert!(r.final_length >= 0.1);
}

fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn video_to_emoji_end_to_end_when_ffmpeg_present() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg not on PATH");
        return;
    }
    // Build a tiny synthetic 64×64 video via ffmpeg's color source, then run
    // it through video_to_emoji and check the output is a non-empty WebM
    // starting with the EBML signature.
    let input = tempfile::NamedTempFile::with_suffix(".mp4").unwrap();
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=64x64:d=1",
        ])
        .arg(input.path())
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg synth failed");

    let bytes = std::fs::read(input.path()).unwrap();
    let out = video_to_emoji(&bytes).await.unwrap();
    assert!(out.len() > 16);
    assert_eq!(&out[..4], &[0x1A, 0x45, 0xDF, 0xA3], "WebM EBML header");
}

/// Build a small synthetic mp4 of `seconds` seconds at `wxh`, color `c`.
fn synth_video(seconds: u32, w: u32, h: u32, color: &str) -> tempfile::NamedTempFile {
    let f = tempfile::NamedTempFile::with_suffix(".mp4").unwrap();
    let s = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s={w}x{h}:d={seconds}"),
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(f.path())
        .status()
        .unwrap();
    assert!(s.success(), "synth failed");
    f
}

fn is_webm(b: &[u8]) -> bool {
    b.len() > 4 && b[..4] == [0x1A, 0x45, 0xDF, 0xA3]
}

#[tokio::test]
async fn video_to_sticker_default_when_ffmpeg_present() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg not on PATH");
        return;
    }
    let mp4 = synth_video(2, 320, 240, "blue");
    let bytes = std::fs::read(mp4.path()).unwrap();
    let out = video_to_sticker(
        &bytes,
        VideoQuoteArguments::default(),
        VideoPreprocess::Default,
    )
    .await
    .unwrap();
    assert!(is_webm(&out), "default preprocess produced non-WebM output");
    assert!(out.len() > 100);
}

#[tokio::test]
async fn video_to_sticker_circle_with_speed_when_ffmpeg_present() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg not on PATH");
        return;
    }
    // 3-second video, ask for 2x speed — exercises alphamerge + setpts.
    let mp4 = synth_video(3, 240, 240, "green");
    let bytes = std::fs::read(mp4.path()).unwrap();
    let args = VideoQuoteArguments {
        speed: Some(2.0),
        ..Default::default()
    };
    let out = video_to_sticker(&bytes, args, VideoPreprocess::Circle)
        .await
        .unwrap();
    assert!(is_webm(&out));
}

#[tokio::test]
async fn video_to_sticker_with_bubble_overlay_when_ffmpeg_present() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg not on PATH");
        return;
    }
    let mp4 = synth_video(2, 320, 240, "red");
    let bytes = std::fs::read(mp4.path()).unwrap();
    let args = VideoQuoteArguments {
        speech_bubble: Some(0),
        ..Default::default()
    };
    let out = video_to_sticker(&bytes, args, VideoPreprocess::Default)
        .await
        .unwrap();
    assert!(is_webm(&out));
}

#[tokio::test]
async fn video_to_sticker_reverse_with_trim_when_ffmpeg_present() {
    if !ffmpeg_available() {
        eprintln!("skipping: ffmpeg not on PATH");
        return;
    }
    let mp4 = synth_video(4, 320, 240, "orange");
    let bytes = std::fs::read(mp4.path()).unwrap();
    let args = VideoQuoteArguments {
        starting_point: Some(1),
        end_point: Some(3),
        reverse: Some(true),
        ..Default::default()
    };
    let out = video_to_sticker(&bytes, args, VideoPreprocess::Default)
        .await
        .unwrap();
    assert!(is_webm(&out));
}
