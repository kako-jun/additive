//! ADDITIVE-13 CLI.
//!
//! Renders a transition between two images. It emits PNG stills / frame sequences
//! via the selected renderer, and bakes opaque `mp4` / `webm` video via ffmpeg
//! (#3). The output kind is inferred from `--output`'s extension: `.png` is a
//! single debug frame, `.mp4` / `.webm` are baked clips.

mod video;

use std::path::{Path, PathBuf};

use additive_core::{all, by_name, timeline, GpuRenderer, OrbDissolve, Transition};
use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use image::imageops::FilterType;
use image::RgbaImage;

use video::{calc_frame_count, render_video, VideoCodec, DEFAULT_DURATION_MS, DEFAULT_FPS};

/// Which renderer backend to drive.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Renderer {
    /// Reference CPU renderer (the parity oracle).
    Cpu,
    /// Production wgpu (Rust + WGSL) renderer; falls back to CPU if no GPU.
    Gpu,
}

#[derive(Parser)]
#[command(
    name = "additive",
    version,
    about = "Render transitions between two images (ADDITIVE-13)"
)]
struct Cli {
    /// First image, shown at t = 0.
    #[arg(long, value_name = "PATH")]
    from: Option<PathBuf>,

    /// Second image, shown at t = 1.
    #[arg(long, value_name = "PATH")]
    to: Option<PathBuf>,

    /// Transition name (see --list).
    #[arg(long, default_value = "crossfade")]
    transition: String,

    /// Output path. The extension picks the mode: `.png` = single debug frame,
    /// `.mp4` = baked H.264, `.webm` = baked VP9.
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Normalized time of the single `.png` output frame, 0.0..=1.0.
    #[arg(long, default_value_t = 0.5)]
    t: f32,

    /// Clip length in milliseconds for video output (`.mp4` / `.webm`).
    #[arg(long, default_value_t = DEFAULT_DURATION_MS)]
    duration_ms: u64,

    /// Frame rate for video output (`.mp4` / `.webm`).
    #[arg(long, default_value_t = DEFAULT_FPS)]
    fps: u32,

    /// Request a transparent overlay clip (alpha). Not yet implemented; see
    /// roadmap #3 follow-up.
    #[arg(long)]
    alpha: bool,

    /// Instead of one frame, write this many PNG frames over [0,1] into --out-dir.
    #[arg(long)]
    frames: Option<u32>,

    /// Output directory for --frames.
    #[arg(long, value_name = "DIR")]
    out_dir: Option<PathBuf>,

    /// List available transitions and exit.
    #[arg(long)]
    list: bool,

    /// Renderer backend: the production `gpu` (wgpu) path or the `cpu` reference.
    #[arg(long, value_enum, default_value_t = Renderer::Gpu)]
    renderer: Renderer,
}

/// Renders frames with the selected backend, falling back from GPU to CPU when
/// no adapter is available.
enum FrameRenderer {
    Cpu,
    Gpu(GpuRenderer),
}

impl FrameRenderer {
    /// Build the requested renderer. `Gpu` falls back to `Cpu` (with a warning)
    /// when no GPU adapter can be acquired.
    fn select(choice: Renderer) -> Self {
        match choice {
            Renderer::Cpu => FrameRenderer::Cpu,
            Renderer::Gpu => match GpuRenderer::new() {
                Some(gpu) => {
                    eprintln!("using gpu renderer (adapter: {})", gpu.adapter_name());
                    FrameRenderer::Gpu(gpu)
                }
                None => {
                    eprintln!("warning: no GPU adapter available; falling back to cpu renderer");
                    FrameRenderer::Cpu
                }
            },
        }
    }

    fn render(&self, tr: &dyn Transition, from: &RgbaImage, to: &RgbaImage, t: f32) -> RgbaImage {
        match self {
            FrameRenderer::Cpu => tr.render_cpu(from, to, t),
            FrameRenderer::Gpu(gpu) => {
                // No.13 orb-dissolve drives the orb-array GPU path; everything
                // else uses the plain from/to/t crossfade-style pipeline.
                if tr.name() == "orb-dissolve" {
                    let pool = OrbDissolve::orb_pool(from);
                    let orbs = OrbDissolve::gpu_orbs(&pool, t);
                    gpu.render_orbs(from, to, tr.shader_wgsl(), t, &orbs)
                } else {
                    gpu.render(from, to, tr.shader_wgsl(), t)
                }
            }
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.list {
        for tr in all() {
            println!(
                "{:>6}  {:<14} {}",
                tr.designation(),
                tr.name(),
                tr.description()
            );
        }
        return Ok(());
    }

    let from_path = cli.from.context("--from is required")?;
    let to_path = cli.to.context("--to is required")?;

    let tr = by_name(&cli.transition)
        .with_context(|| format!("unknown transition '{}'; run --list", cli.transition))?;

    let renderer = FrameRenderer::select(cli.renderer);

    let from = image::open(&from_path)
        .with_context(|| format!("opening {}", from_path.display()))?
        .to_rgba8();
    let (w, h) = from.dimensions();

    let to = image::open(&to_path)
        .with_context(|| format!("opening {}", to_path.display()))?
        .to_rgba8();
    // Match `to` to `from`'s dimensions so the per-pixel transition is defined.
    let to = if to.dimensions() == (w, h) {
        to
    } else {
        image::imageops::resize(&to, w, h, FilterType::Lanczos3)
    };

    match (cli.frames, cli.out_dir.as_deref()) {
        (Some(n), Some(dir)) => {
            if n < 2 {
                bail!("--frames must be >= 2");
            }
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
            for (i, t) in timeline(n).enumerate() {
                let frame = renderer.render(tr.as_ref(), &from, &to, t);
                let path = dir.join(format!("frame_{i:04}.png"));
                frame
                    .save(&path)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            eprintln!("wrote {n} frames to {}", dir.display());
        }
        (Some(_), None) => bail!("--frames requires --out-dir"),
        (None, _) => {
            let output = cli
                .output
                .as_deref()
                .context("--output (or --frames + --out-dir) is required")?;
            let opts = OutputOpts {
                t: cli.t,
                duration_ms: cli.duration_ms,
                fps: cli.fps,
                alpha: cli.alpha,
            };
            run_output(&opts, tr.as_ref(), &renderer, &from, &to, output)?;
        }
    }

    Ok(())
}

/// Knobs for a single `--output`, lifted off [`Cli`] so dispatch doesn't borrow
/// the whole (partially-moved) parsed struct.
struct OutputOpts {
    t: f32,
    duration_ms: u64,
    fps: u32,
    alpha: bool,
}

/// Dispatch a single `--output` by its extension: a `.png` debug frame, a baked
/// `.mp4` / `.webm` clip, or (eventually) an alpha `.mov`.
fn run_output(
    opts: &OutputOpts,
    tr: &dyn Transition,
    renderer: &FrameRenderer,
    from: &RgbaImage,
    to: &RgbaImage,
    output: &Path,
) -> Result<()> {
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    // Alpha overlay (.mov / --alpha) is a deliberate follow-up: it needs a
    // straight-alpha overlay render path (skip the `to` background, emit
    // from+effect on transparent) in WGSL *and* the CPU oracle, which is a core
    // change with its own parity story. Baked mp4/webm ships first (#3); alpha is
    // tracked as a #3 follow-up. Fail loudly rather than silently bake opaque.
    if opts.alpha || ext.as_deref() == Some("mov") {
        bail!(
            "alpha overlay output (--alpha / .mov) is not implemented yet; it needs a \
             straight-alpha overlay render path in additive-core (WGSL + CPU oracle) and \
             is tracked as a #3 follow-up. Use .mp4 or .webm for a baked (opaque) clip."
        );
    }

    if let Some(codec) = VideoCodec::from_path(output) {
        let total = calc_frame_count(opts.duration_ms, opts.fps);
        render_video(output, codec, total, opts.fps, |_, t| {
            renderer.render(tr, from, to, t)
        })
        .with_context(|| format!("encoding {}", output.display()))?;
        eprintln!(
            "wrote {} ({total} frames @ {}fps, {}ms)",
            output.display(),
            opts.fps,
            opts.duration_ms,
        );
        return Ok(());
    }

    // Default / `.png`: single debug frame at --t.
    let frame = renderer.render(tr, from, to, opts.t);
    frame
        .save(output)
        .with_context(|| format!("writing {}", output.display()))?;
    eprintln!("wrote {} at t={}", output.display(), opts.t);
    Ok(())
}
