//! ADDITIVE-13 CLI.
//!
//! Renders a transition between two images. For now it emits PNG stills / frame
//! sequences via the reference CPU renderer; video muxing (mp4 / alpha mov) and
//! the wgpu fast path land in issues #1 and #3.

use std::path::PathBuf;

use additive_core::{all, by_name, timeline};
use anyhow::{bail, Context, Result};
use clap::Parser;
use image::imageops::FilterType;

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
                let frame = tr.render_cpu(&from, &to, t);
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
            let frame = tr.render_cpu(&from, &to, cli.t);
            frame
                .save(&output)
                .with_context(|| format!("writing {}", output.display()))?;
            eprintln!("wrote {} at t={}", output.display(), cli.t);
        }
    }

    Ok(())
}
