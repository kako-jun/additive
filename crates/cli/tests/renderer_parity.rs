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

/// Run the built CLI with the given renderer, writing a single frame at `t`.
fn run_renderer(from: &Path, to: &Path, output: &Path, renderer: &str, t: &str) {
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
        ])
        .status()
        .expect("failed to spawn additive binary");
    assert!(
        status.success(),
        "additive --renderer {renderer} exited non-zero"
    );
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

    run_renderer(&from_path, &to_path, &cpu_path, "cpu", "0.5");
    run_renderer(&from_path, &to_path, &gpu_path, "gpu", "0.5");

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
