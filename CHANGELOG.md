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
- **`No.13 — orb-dissolve` (flagship), conveyor sweep-wipe (#14)**: a band of
  color orbs (palette via `orber-core` clustering, per-orb color sampled from
  `from`), flowing on orber's one-way conveyor, **sweeps across** the frame. The
  base is a directional `from → to` step at a **wipe front** `p(t)` that travels
  off-frame at both ends: ahead of the band the base is still `from`, behind it
  `to`, and the `from→to` seam always rides hidden **inside** the band — never a
  raw visible boundary. It is a directional wipe, not a translucent cross-fade and
  not a global occlusion pulse (the orbs cover one perpendicular slice at a time,
  never the whole frame). Coverage is geometric: orbs tile a jittered grid whose
  cross-axis rows span the band width (gap-free) and whose flow-axis columns give
  the band depth, centered on the front and riding a one-way drift; orb distance
  is aspect-corrected so discs read isotropically on non-square frames. Runs on
  both the CPU oracle and the WGSL GPU path (`MAX_ORBS` 128,
  `OrbParams{t,orb_count,aspect_x,aspect_y,front,dir_code}` + orb-array uniform);
  tests pin the mechanism on both paths — t=0/t=1 endpoints, **monotone** growth
  of the `to` region (the front never retreats), and **seam coverage** (the
  boundary is hidden under the band). Strict CPU↔GPU pixel parity is intentionally
  not asserted; the mechanism is. No.0 crossfade's parity stays untouched.
- **No.13 knobs (#14)**: `--count` (orb count, 1..=128), `--speed` (conveyor
  drift), `--direction` (`lr`/`rl`/`tb`/`bt`), `--orb-size` (disc-size multiplier)
  — all orb-dissolve-only and harmless to other transitions.
- CLI: `--from / --to / --transition / --output / --t`, `--duration-ms / --fps`,
  `--frames / --out-dir`, `--list`, `--count / --speed / --direction / --orb-size`.
- wasm bindings exposing the CPU reference renderer for an immediate (pre-wgpu)
  browser preview.
- Documentation: README (EN), `docs/overview.md` (EN), `docs/roadmap.md` (JP),
  `CLAUDE.md` (JP), `DESIGN.md`.

### Changed
- **No.13 reworked from a global occlusion pulse to a conveyor sweep-wipe (#14)**:
  the earlier mechanism grew the orbs to cover the *whole* frame at `t≈0.5` and
  hard-swapped the base globally underneath. That read as a flash, not a wipe. The
  corrected mechanism keeps the conveyor flow but concentrates the orbs into a
  band that sweeps one way across the frame, with the `from→to` seam hidden inside
  the moving band. The occupancy/occlusion tests (which only hold for the old
  global model) are replaced by sweep-monotonicity and seam-coverage tests.

### Notes
- The production renderer is **wgpu** (Rust + WGSL, one source for native CLI and
  browser, identical output); it lands in #1. The CPU renderer here is the
  reference path, not the production path.
