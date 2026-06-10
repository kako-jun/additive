//! End-to-end renderer parity through the built `additive` binary.
//!
//! Drives the actual CLI (via `CARGO_BIN_EXE_additive`) with `--renderer cpu`
//! and `--renderer gpu` on the same inputs and asserts the two output PNGs match
//! within `±2`/channel — the same tolerance the in-process `gpu::tests` use.
//!
//! On a machine with no GPU adapter the `gpu` run falls back to the CPU path by
//! design, so the comparison is then trivially satisfied. That is fine: the test
//! exercises the CLI wiring (flag parsing, backend selection, PNG I/O) either way.

use std::path::Path;
use std::process::Command;

use image::{Rgba, RgbaImage};

/// A small, varied image so parity isn't trivially satisfied by a flat color.
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

/// Run the built CLI with the given renderer + transition, writing a frame at `t`.
fn run_renderer(from: &Path, to: &Path, output: &Path, renderer: &str, t: &str, transition: &str) {
    let status = Command::new(env!("CARGO_BIN_EXE_additive"))
        .args([
            "--from",
            from.to_str().unwrap(),
            "--to",
            to.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--renderer",
            renderer,
            "--t",
            t,
            "--transition",
            transition,
        ])
        .status()
        .expect("failed to spawn additive binary");
    assert!(
        status.success(),
        "additive --renderer {renderer} --transition {transition} exited non-zero"
    );
}

/// Max per-channel diff between two same-sized frames.
fn max_channel_diff(a: &RgbaImage, b: &RgbaImage) -> u8 {
    assert_eq!(a.dimensions(), b.dimensions(), "output sizes differ");
    let mut max_diff = 0u8;
    for (x, y, ap) in a.enumerate_pixels() {
        let bp = b.get_pixel(x, y);
        for ch in 0..4 {
            max_diff = max_diff.max(ap.0[ch].abs_diff(bp.0[ch]));
        }
    }
    max_diff
}

#[test]
fn cli_cpu_and_gpu_outputs_match() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let from_path = dir.path().join("from.png");
    let to_path = dir.path().join("to.png");
    let cpu_path = dir.path().join("cpu.png");
    let gpu_path = dir.path().join("gpu.png");

    // 16x16 keeps the test cheap while still exercising real pixels.
    gradient(16, 16, [10, 40, 80, 255])
        .save(&from_path)
        .expect("write from.png");
    gradient(16, 16, [200, 90, 20, 200])
        .save(&to_path)
        .expect("write to.png");

    run_renderer(&from_path, &to_path, &cpu_path, "cpu", "0.5", "crossfade");
    run_renderer(&from_path, &to_path, &gpu_path, "gpu", "0.5", "crossfade");

    let cpu = image::open(&cpu_path).expect("open cpu.png").to_rgba8();
    let gpu = image::open(&gpu_path).expect("open gpu.png").to_rgba8();
    assert_eq!(cpu.dimensions(), gpu.dimensions(), "output sizes differ");

    let mut max_diff = 0u8;
    for (x, y, cp) in cpu.enumerate_pixels() {
        let gp = gpu.get_pixel(x, y);
        for ch in 0..4 {
            let d = cp.0[ch].abs_diff(gp.0[ch]);
            max_diff = max_diff.max(d);
            assert!(
                d <= 2,
                "pixel ({x},{y}) channel {ch} differs by {d} (cpu={:?} gpu={:?})",
                cp.0,
                gp.0
            );
        }
    }
    eprintln!("cli renderer parity: max per-channel diff = {max_diff}");
}

/// **No.14 aqua-dissolve — relaxed renderer parity.** Like orb-dissolve, the spiral
/// bleed uses `sin`, whose CPU and GPU implementations differ by ULPs and compound
/// over 48 taps, so this asserts the *mechanism* / closeness rather than strict ±2
/// parity. The **endpoints** (t=0 / t=1) must match within ±2 (off-frame front ⇒ a
/// flat mix the bleed doesn't perturb), exactly like crossfade; the **mid-clip**
/// frame must merely be *close* (mean per-channel diff small), since the dithered
/// watercolor band legitimately diverges per-pixel.
///
/// On a GPU-less machine the gpu run falls back to CPU and every leg is trivially
/// satisfied — the test still exercises the full CLI aqua-dissolve wiring.
#[test]
fn cli_aqua_dissolve_relaxed_parity() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let from_path = dir.path().join("from.png");
    let to_path = dir.path().join("to.png");

    gradient(48, 32, [10, 40, 80, 255])
        .save(&from_path)
        .expect("write from.png");
    gradient(48, 32, [200, 90, 20, 255])
        .save(&to_path)
        .expect("write to.png");

    // Endpoints: the front is off-frame, so the mix is a flat from/to the bleed
    // cannot perturb — strict ±2 parity holds just like crossfade.
    for t in ["0.0", "1.0"] {
        let cpu_path = dir.path().join(format!("cpu_{t}.png"));
        let gpu_path = dir.path().join(format!("gpu_{t}.png"));
        run_renderer(&from_path, &to_path, &cpu_path, "cpu", t, "aqua-dissolve");
        run_renderer(&from_path, &to_path, &gpu_path, "gpu", t, "aqua-dissolve");
        let cpu = image::open(&cpu_path).expect("open cpu").to_rgba8();
        let gpu = image::open(&gpu_path).expect("open gpu").to_rgba8();
        let d = max_channel_diff(&cpu, &gpu);
        eprintln!("aqua-dissolve endpoint t={t}: max per-channel diff = {d}");
        assert!(
            d <= 2,
            "aqua-dissolve endpoint t={t} must match within ±2 (got {d})"
        );
    }

    // Mid-clip: the dithered watercolor band diverges per-pixel between CPU/GPU
    // `sin`, so assert closeness in the *mean* (a sane bound for a relaxed effect),
    // not strict ±2.
    let cpu_path = dir.path().join("cpu_mid.png");
    let gpu_path = dir.path().join("gpu_mid.png");
    run_renderer(
        &from_path,
        &to_path,
        &cpu_path,
        "cpu",
        "0.5",
        "aqua-dissolve",
    );
    run_renderer(
        &from_path,
        &to_path,
        &gpu_path,
        "gpu",
        "0.5",
        "aqua-dissolve",
    );
    let cpu = image::open(&cpu_path).expect("open cpu").to_rgba8();
    let gpu = image::open(&gpu_path).expect("open gpu").to_rgba8();
    assert_eq!(cpu.dimensions(), gpu.dimensions(), "output sizes differ");
    let mut sum = 0u64;
    let mut n = 0u64;
    for (x, y, cp) in cpu.enumerate_pixels() {
        let gp = gpu.get_pixel(x, y);
        for ch in 0..3 {
            sum += cp.0[ch].abs_diff(gp.0[ch]) as u64;
            n += 1;
        }
    }
    let mean = sum as f32 / n as f32;
    eprintln!("aqua-dissolve mid-clip: mean per-channel diff = {mean:.2}");
    assert!(
        mean < 12.0,
        "aqua-dissolve mid-clip CPU/GPU must stay close in the mean (got {mean:.2})"
    );
}
