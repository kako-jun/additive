//! End-to-end video-output regression tests through the built `additive` binary.
//!
//! These drive the real CLI (via `CARGO_BIN_EXE_additive`) to bake actual clips,
//! so they require `ffmpeg` (and, for codec assertions, `ffprobe`) on `PATH`. When
//! either is missing the test **skips** (prints a notice and returns) rather than
//! failing — same gating philosophy as `renderer_parity` on GPU-less machines.
//!
//! Coverage:
//! - `mp4_from_odd_dimensions_encodes`: a 37x23 (odd-on-both-axes) input must still
//!   produce a non-empty `.mp4` with an `h264` stream. This is the regression guard
//!   for the `yuv420p` even-dimension requirement (`-vf scale=trunc(iw/2)*2:...`).
//! - `mov_and_alpha_are_rejected`: `.mov` and `--alpha` must exit non-zero with a
//!   "not implemented" message, so the unimplemented alpha path can never silently
//!   bake an opaque clip instead.

use std::path::Path;
use std::process::Command;

use image::{Rgba, RgbaImage};

/// True if `tool` is runnable (used to gate on ffmpeg/ffprobe availability).
fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A small, varied image so the encoder sees real (non-flat) pixels.
fn gradient(w: u32, h: u32, base: [u8; 4]) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels_mut() {
        *px = Rgba([
            base[0].wrapping_add((x * 7) as u8),
            base[1].wrapping_add((y * 11) as u8),
            base[2].wrapping_add((x * y) as u8),
            base[3],
        ]);
    }
    img
}

#[test]
fn mp4_from_odd_dimensions_encodes() {
    if !tool_available("ffmpeg") {
        eprintln!("skipping mp4_from_odd_dimensions_encodes: ffmpeg not on PATH");
        return;
    }

    let dir = tempfile::tempdir().expect("create tempdir");
    let from_path = dir.path().join("from.png");
    let to_path = dir.path().join("to.png");
    let out_path = dir.path().join("odd.mp4");

    // 37x23: odd on *both* axes — the exact case that breaks yuv420p without the
    // even-rounding scale filter.
    gradient(37, 23, [10, 40, 80, 255])
        .save(&from_path)
        .expect("write from.png");
    gradient(37, 23, [200, 90, 20, 255])
        .save(&to_path)
        .expect("write to.png");

    let status = Command::new(env!("CARGO_BIN_EXE_additive"))
        .args([
            "--from",
            from_path.to_str().unwrap(),
            "--to",
            to_path.to_str().unwrap(),
            "--transition",
            "orb-dissolve",
            "--output",
            out_path.to_str().unwrap(),
            "--duration-ms",
            "500",
            "--fps",
            "24",
            "--renderer",
            "cpu",
        ])
        .status()
        .expect("failed to spawn additive binary");
    assert!(
        status.success(),
        "additive odd-dimension mp4 exited non-zero"
    );

    let meta = std::fs::metadata(&out_path).expect("output mp4 should exist");
    assert!(meta.len() > 0, "output mp4 should be non-empty");

    // If ffprobe is around, confirm the stream is actually h264 and rounded even.
    if tool_available("ffprobe") {
        let (codec, w, h) = ffprobe_stream(&out_path);
        assert_eq!(codec, "h264", "codec should be h264, got {codec}");
        assert_eq!(w % 2, 0, "width must be even, got {w}");
        assert_eq!(h % 2, 0, "height must be even, got {h}");
        // 37 -> 36, 23 -> 22 (rounded down to even).
        assert_eq!((w, h), (36, 22), "odd dims should round down to (36,22)");
    }
}

/// Run `ffprobe` on the first video stream, returning (codec_name, width, height).
fn ffprobe_stream(path: &Path) -> (String, u32, u32) {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height",
            "-of",
            "default=noprint_wrappers=1:nokey=0",
        ])
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert!(out.status.success(), "ffprobe failed");
    let text = String::from_utf8_lossy(&out.stdout);

    let mut codec = String::new();
    let mut w = 0u32;
    let mut h = 0u32;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("codec_name=") {
            codec = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("width=") {
            w = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("height=") {
            h = v.trim().parse().unwrap_or(0);
        }
    }
    (codec, w, h)
}

#[test]
fn mov_and_alpha_are_rejected() {
    // No ffmpeg dependency: these must be rejected *before* any encode attempt.
    let dir = tempfile::tempdir().expect("create tempdir");
    let from_path = dir.path().join("from.png");
    let to_path = dir.path().join("to.png");
    gradient(16, 16, [10, 40, 80, 255])
        .save(&from_path)
        .expect("write from.png");
    gradient(16, 16, [200, 90, 20, 255])
        .save(&to_path)
        .expect("write to.png");

    // (a) .mov output is rejected.
    let mov_out = dir.path().join("clip.mov");
    let out = Command::new(env!("CARGO_BIN_EXE_additive"))
        .args([
            "--from",
            from_path.to_str().unwrap(),
            "--to",
            to_path.to_str().unwrap(),
            "--output",
            mov_out.to_str().unwrap(),
        ])
        .output()
        .expect("spawn additive (.mov)");
    assert!(!out.status.success(), ".mov output must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not implemented"),
        ".mov stderr must say 'not implemented', got: {stderr}"
    );
    assert!(
        !mov_out.exists(),
        ".mov rejection must not write any output file"
    );

    // (b) --alpha (with an otherwise-valid .mp4 target) is rejected too.
    let alpha_out = dir.path().join("alpha.mp4");
    let out = Command::new(env!("CARGO_BIN_EXE_additive"))
        .args([
            "--from",
            from_path.to_str().unwrap(),
            "--to",
            to_path.to_str().unwrap(),
            "--output",
            alpha_out.to_str().unwrap(),
            "--alpha",
        ])
        .output()
        .expect("spawn additive (--alpha)");
    assert!(!out.status.success(), "--alpha must exit non-zero");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not implemented"),
        "--alpha stderr must say 'not implemented', got: {stderr}"
    );
}
