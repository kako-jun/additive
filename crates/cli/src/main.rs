//! ADDITIVE-13 CLI.
//!
//! Renders a transition between two images. For now it emits PNG stills / frame
//! sequences via the reference CPU renderer; video muxing (mp4 / alpha mov) and
//! the wgpu fast path land in issues #1 and #3.

use std::path::PathBuf;

use additive_core::{all, by_name, timeline, GpuRenderer, Transition};
use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use image::imageops::FilterType;
use image::RgbaImage;

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

    /// Output PNG path for a single frame.
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Normalized time of the single output frame, 0.0..=1.0.
    #[arg(long, default_value_t = 0.5)]
    t: f32,

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
            FrameRenderer::Gpu(gpu) => gpu.render(from, to, tr.shader_wgsl(), t),
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

    match (cli.frames, cli.out_dir) {
        (Some(n), Some(dir)) => {
            if n < 2 {
                bail!("--frames must be >= 2");
            }
            std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
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
                .context("--output (or --frames + --out-dir) is required")?;
            let frame = renderer.render(tr.as_ref(), &from, &to, cli.t);
            frame
                .save(&output)
                .with_context(|| format!("writing {}", output.display()))?;
            eprintln!("wrote {} at t={}", output.display(), cli.t);
        }
    }

    Ok(())
}
