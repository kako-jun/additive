# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog, and this project adheres to Semantic Versioning.

## [Unreleased]

### Added
- Initial scaffold (session549). Cargo workspace mirroring orber: `additive-core`
  (pure transition core, wasm-buildable), `additive` (CLI), `additive-wasm`
  (browser bindings).
- `Transition` contract — `(from, to, t) -> RGBA frame` — with an E-number style
  designation per additive.
- `No.0 — crossfade`: the reference CPU renderer and parity oracle.
- **wgpu renderer (native) behind the `gpu` feature (#1)**: `GpuRenderer` (wgpu 29)
  renders `No.0 crossfade` in WGSL on a headless GPU; CLI gains `--renderer cpu|gpu`
  (gpu default, CPU fallback). A parity test confirms GPU output matches the CPU
  oracle within ±2/channel on a real adapter. Textures are `Rgba8Unorm` (never
  srgb) and readback handles 256-byte row padding, so the GPU `mix` equals the CPU
  integer lerp. wasm builds keep the feature off (browser/WebGPU wiring is #4).
- **Baked video output via ffmpeg (#3)**: `--output out.mp4` / `out.webm` renders
  the whole transition as a frame sequence and muxes it with `ffmpeg` —
  `.mp4` → H.264 (`yuv420p`, `+faststart`), `.webm` → VP9 (`yuv420p`). New
  `--duration-ms` (default 2500) and `--fps` (default 30) control clip length; the
  output kind is inferred from `--output`'s extension (`.png` stays the single
  debug frame). I/O and the child process live in `crates/cli/src/video.rs` so
  `additive-core` stays pure. ffmpeg missing from `PATH` gives a clear error with
  an install hint; a non-zero ffmpeg exit surfaces its stderr. Alpha overlay
  output (`--alpha` / `.mov`) is reserved but rejected with a not-yet-implemented
  error — a #3 follow-up needing a straight-alpha overlay render path in core
  (WGSL + CPU oracle).
- CLI: `--from / --to / --transition / --output / --t`, `--duration-ms / --fps`,
  `--frames / --out-dir`, `--list`.
- wasm bindings exposing the CPU reference renderer for an immediate (pre-wgpu)
  browser preview.
- Documentation: README (EN), `docs/overview.md` (EN), `docs/roadmap.md` (JP),
  `CLAUDE.md` (JP), `DESIGN.md`.

### Notes
- The production renderer is **wgpu** (Rust + WGSL, one source for native CLI and
  browser, identical output); it lands in #1. The CPU renderer here is the
  reference path, not the production path.
