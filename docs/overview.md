# ADDITIVE-13 overview

ADDITIVE-13 is a catalogue of image **transitions**. Each transition ("additive")
maps two same-sized images and a normalized time `t` to a single RGBA frame:

```
(from, to, t in 0.0..=1.0) -> RGBA frame
```

`t = 0` is pure `from`, `t = 1` is pure `to`. A clip is just that function sampled
over a timeline.

## The additive catalogue

Each additive has an E-number style designation and a stable kebab-case name:

| Designation | Name           | Status      | Notes                                                        |
| ----------- | -------------- | ----------- | ------------------------------------------------------------ |
| No.0        | `crossfade`    | implemented | Linear cross-dissolve. Reference baseline + parity oracle.   |
| No.13       | `orb-dissolve` | implemented | `from` shatters into drifting orbs (via orber-core) and clears to reveal `to`. Flagship. |

Later additives (ink-bleed, light-leak, glitch, …) plug into the same contract.

## Renderer

A transition has to satisfy two requirements at once:

1. **Real-time preview in the browser** — every additive needs a live preview,
   exactly like orber.
2. **The CLI and the browser produce identical output** — what you preview is
   what you render to a file.

These two together eliminate every option but one:

| Approach                                  | Real-time | CLI == web |
| ----------------------------------------- | --------- | ---------- |
| CPU rasterizer compiled to wasm           | no        | yes        |
| CPU CLI + a separate WebGL/GLSL preview   | yes       | **no** (two renderers, two results) |
| **wgpu (Rust + WGSL), native + browser**  | **yes**   | **yes**    |

So the production renderer is **wgpu**: the rendering is written once in Rust and
WGSL and runs on the GPU in the browser (WebGPU, WebGL2 fallback) and headless on
the desktop for the CLI. Because both run the same WGSL, results are visually
identical (bit-exactness across different GPUs is not guaranteed, but the
algorithm is one and the same — a different category from maintaining two
implementations).

> Raw WebGPU written in JavaScript would not solve this: the shader would live
> outside Rust, recreating the same two-codebase split. The point of wgpu is that
> the shader lives *inside* the Rust crate and serves both targets.

The CPU renderer in `additive-core` exists as the **reference / parity oracle**
the wgpu path is verified against, and to give the web GUI a working (if slower)
preview before the wgpu path lands.

**Status (#1):** the native wgpu path is implemented behind the `gpu` cargo
feature in `additive-core`. `No.0 crossfade` runs in WGSL, and a parity test
confirms it matches the CPU oracle within ±2/channel on a real GPU (verified on
Intel ADL-N via Vulkan; falls back to lavapipe or skips gracefully where no
adapter exists). The CLI selects it by default (`--renderer cpu|gpu`, gpu
default, CPU fallback on adapter failure). The browser / WebGPU half lands in #4;
`additive-core` and `additive-wasm` still build for `wasm32` with the feature off.

## Output modes

- **Baked** — composite `from → to` internally and emit an opaque `mp4` / `webm`.
  Simplest; this is what validates the look first. **Implemented (#3):** the CLI
  renders the whole transition as a frame sequence (`--duration-ms`, `--fps`) and
  shells out to `ffmpeg` to mux it — `.mp4` → H.264/`yuv420p`, `.webm` →
  VP9/`yuv420p`. I/O and the child process live in `crates/cli/src/video.rs` so
  `additive-core` stays a pure `(from, to, t) -> RGBA frame` library; the output
  kind is inferred from `--output`'s extension. ffmpeg missing from `PATH` is a
  clear error with an install hint.
- **Alpha overlay** — emit only the dissolving layer with an alpha channel, so
  name-name (or any compositor) can play it over live content. The browser path
  reuses orber's hard-won approach: PNG frames muxed into a QuickTime `.mov`
  container (RGBA 32-bit lossless, NLE-compatible) rather than fighting
  ffmpeg.wasm's vp9-alpha — see orber's `web/src/lib/movMuxer.ts`. **Follow-up
  (#3):** this needs a straight-alpha *overlay* render path in core — skip the
  `to` background and emit `from`+effect on transparent — in **both** the WGSL
  shader and the CPU oracle, a core change with its own parity story. The CLI
  already reserves `--alpha` / `.mov` and rejects them with a clear
  not-yet-implemented error rather than silently baking opaque.

## Relationship to orber

`No.13 — orb-dissolve` reuses [orber](https://github.com/kako-jun/orber)'s orb
engine (`orber-core`: color clustering → orb placement) so it does not reinvent
orbs. orber is the *supplier*; ADDITIVE-13 is a *consumer*. orber itself stays a
single-input stylizer — no `--transition` mode is bolted onto it.
