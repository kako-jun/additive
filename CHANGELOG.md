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
- CLI: `--from / --to / --transition / --output / --t`, `--frames / --out-dir`,
  `--list`.
- wasm bindings exposing the CPU reference renderer for an immediate (pre-wgpu)
  browser preview.
- Documentation: README (EN), `docs/overview.md` (EN), `docs/roadmap.md` (JP),
  `CLAUDE.md` (JP), `DESIGN.md`.

### Notes
- The production renderer is **wgpu** (Rust + WGSL, one source for native CLI and
  browser, identical output); it lands in #1. The CPU renderer here is the
  reference path, not the production path.
