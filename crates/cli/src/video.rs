//! Baked (opaque) video output.
//!
//! The CLI renders the whole transition as a frame sequence, then shells out to
//! `ffmpeg` to mux the PNGs into an opaque `mp4` (H.264) or `webm` (VP9). I/O and
//! the child process live here, on the CLI side, so `additive-core` stays a pure
//! `(from, to, t) -> RGBA frame` library (this mirrors orber's `cli/src/video.rs`).
//!
//! # Design notes
//!
//! - Frames are written to a [`tempfile::TempDir`] as `frame_%05d.png`, then
//!   `ffmpeg -framerate {fps} -i frame_%05d.png ...` muxes them. The temp dir is
//!   dropped (cleaned up) when [`render_video`] returns.
//! - The frame *count* is `duration_ms * fps / 1000`, clamped to at least 2 so a
//!   transition always has both endpoints. Resolution is the `from`/`to` image
//!   size (callers resize `to` to match `from`), not a fixed video size — unlike
//!   orber, additive transitions are defined per source-image dimensions.
//! - mp4 is baked opaque to `yuv420p` (H.264); webm to `yuv420p` VP9. Alpha output
//!   (`.mov`) is intentionally **not** handled here — see the CLI for why it is a
//!   follow-up (it needs a straight-alpha overlay render path in core).
//! - `ffmpeg` missing from `PATH` yields [`VideoError::FfmpegNotFound`] with an
//!   install hint; a non-zero exit yields [`VideoError::FfmpegFailed`] carrying
//!   ffmpeg's stderr.

use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

use image::RgbaImage;

/// Default clip length in milliseconds when `--duration-ms` is omitted.
pub const DEFAULT_DURATION_MS: u64 = 2500;
/// Default frame rate when `--fps` is omitted.
pub const DEFAULT_FPS: u32 = 30;

/// Video codec / container, inferred from the output file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    /// H.264 (libx264) in an `mp4` container, baked opaque (`yuv420p`).
    H264,
    /// VP9 (libvpx-vp9) in a `webm` container, baked opaque (`yuv420p`).
    Vp9,
}

impl VideoCodec {
    /// Infer the codec from an output path's extension.
    ///
    /// `.mp4` -> [`VideoCodec::H264`], `.webm` -> [`VideoCodec::Vp9`]. Anything
    /// else (including `.png` stills and `.mov` alpha clips) is `None`; the caller
    /// decides what to do with non-video extensions.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "mp4" => Some(VideoCodec::H264),
            "webm" => Some(VideoCodec::Vp9),
            _ => None,
        }
    }
}

/// Errors from [`render_video`].
#[derive(Debug)]
pub enum VideoError {
    /// The `ffmpeg` binary was not found in `PATH`.
    FfmpegNotFound,
    /// `ffmpeg` exited non-zero; carries its stderr for diagnosis.
    FfmpegFailed { status: ExitStatus, stderr: String },
    /// I/O failure (temp dir creation, PNG write, spawning ffmpeg, …).
    Io(io::Error),
    /// A frame failed to encode to PNG.
    FrameSave(image::ImageError),
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FfmpegNotFound => write!(
                f,
                "ffmpeg not found in PATH; install ffmpeg (e.g. `apt install ffmpeg` / `brew install ffmpeg`) and retry"
            ),
            Self::FfmpegFailed { status, stderr } => {
                write!(f, "ffmpeg failed with {status}: {stderr}")
            }
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::FrameSave(e) => write!(f, "failed to encode frame: {e}"),
        }
    }
}

impl std::error::Error for VideoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::FrameSave(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for VideoError {
    fn from(e: io::Error) -> Self {
        VideoError::Io(e)
    }
}

/// Number of frames for a clip of `duration_ms` at `fps`.
///
/// Computed as `duration_ms * fps / 1000`, clamped to **at least 2** so a clip
/// always covers both transition endpoints (`t = 0` and `t = 1`). Uses saturating
/// arithmetic so absurd inputs cannot overflow into a tiny count.
pub fn calc_frame_count(duration_ms: u64, fps: u32) -> u32 {
    let n = duration_ms.saturating_mul(fps as u64) / 1000;
    n.clamp(2, u32::MAX as u64) as u32
}

/// Render every frame of the transition and mux them into an opaque video.
///
/// `render_frame(i, t)` produces frame `i` (of `total`) at normalized time `t`;
/// the caller wires it to the chosen renderer/transition. Frames are written to a
/// temp dir as `frame_%05d.png`, then `ffmpeg` muxes them into `output` using the
/// codec inferred for that container.
///
/// Progress is printed to stderr (every ~10% and on ffmpeg launch). Callers that
/// want silence should redirect stderr.
pub fn render_video<F>(
    output: &Path,
    codec: VideoCodec,
    total: u32,
    fps: u32,
    mut render_frame: F,
) -> Result<(), VideoError>
where
    F: FnMut(u32, f32) -> RgbaImage,
{
    eprintln!("additive: rendering {total} frames at {fps}fps...");

    let temp_dir = tempfile::TempDir::new()?;

    // `t` spans the closed interval [0, 1] so the clip starts on pure `from` and
    // ends on pure `to` — same convention as `additive_core::timeline`.
    let progress_step = (total / 10).max(1);
    for i in 0..total {
        let t = if total <= 1 {
            0.0
        } else {
            i as f32 / (total - 1) as f32
        };
        let frame = render_frame(i, t);
        let path = temp_dir.path().join(format!("frame_{i:05}.png"));
        frame.save(&path).map_err(VideoError::FrameSave)?;
        if i > 0 && i % progress_step == 0 {
            let pct = (i * 100) / total;
            eprintln!("additive: {pct}% ({i}/{total} frames)");
        }
    }

    eprintln!("additive: invoking ffmpeg ({codec:?})...");
    let pattern = temp_dir.path().join("frame_%05d.png");
    let fps_str = fps.to_string();

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-framerate")
        .arg(&fps_str)
        .arg("-i")
        .arg(&pattern);

    match codec {
        VideoCodec::H264 => {
            cmd.args([
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ]);
        }
        VideoCodec::Vp9 => {
            cmd.args([
                "-c:v",
                "libvpx-vp9",
                "-pix_fmt",
                "yuv420p",
                "-b:v",
                "0",
                "-crf",
                "32",
            ]);
        }
    }

    cmd.arg(output);

    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(VideoError::FfmpegNotFound),
        Err(e) => return Err(VideoError::Io(e)),
    };

    if !out.status.success() {
        return Err(VideoError::FfmpegFailed {
            status: out.status,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn frame_count_math() {
        // 2000ms @ 30fps -> 60
        assert_eq!(calc_frame_count(2000, 30), 60);
        // 1500ms @ 24fps -> 36
        assert_eq!(calc_frame_count(1500, 24), 36);
        // default 2500ms @ 30fps -> 75
        assert_eq!(calc_frame_count(DEFAULT_DURATION_MS, DEFAULT_FPS), 75);
    }

    #[test]
    fn frame_count_floor_is_two() {
        // Anything that would round below 2 is clamped up so both endpoints exist.
        assert_eq!(calc_frame_count(0, 30), 2);
        assert_eq!(calc_frame_count(1, 30), 2);
        assert_eq!(calc_frame_count(33, 30), 2); // 0.99 frames -> 0 -> clamp 2
    }

    #[test]
    fn frame_count_overflow_safe() {
        // Saturating math: extreme inputs must not wrap to a tiny count.
        let n = calc_frame_count(u64::MAX, u32::MAX);
        assert!(n >= 2, "overflow must not produce < 2 frames, got {n}");
    }

    #[test]
    fn codec_from_path_extensions() {
        assert_eq!(
            VideoCodec::from_path(&PathBuf::from("out.mp4")),
            Some(VideoCodec::H264)
        );
        assert_eq!(
            VideoCodec::from_path(&PathBuf::from("OUT.MP4")),
            Some(VideoCodec::H264)
        );
        assert_eq!(
            VideoCodec::from_path(&PathBuf::from("clip.webm")),
            Some(VideoCodec::Vp9)
        );
        assert_eq!(VideoCodec::from_path(&PathBuf::from("peek.png")), None);
        assert_eq!(VideoCodec::from_path(&PathBuf::from("over.mov")), None);
        assert_eq!(VideoCodec::from_path(&PathBuf::from("noext")), None);
    }

    #[test]
    fn ffmpeg_not_found_display_has_install_hint() {
        let msg = format!("{}", VideoError::FfmpegNotFound);
        assert!(msg.contains("ffmpeg"), "should mention ffmpeg: {msg}");
        assert!(msg.contains("install"), "should mention install: {msg}");
    }
}
