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
| No.13       | `orb-dissolve` | implemented | A band of color orbs (palette via orber-core) flows on a one-way conveyor and **sweeps across** the frame: ahead of the band the base is still `from`, behind it `to`, and the `from→to` seam rides hidden inside the band. Flagship. Knobs: `--count` / `--speed` / `--direction` / `--orb-size`. |

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

## No.13 — orb-dissolve (conveyor sweep-wipe)

No.13 is a **directional sweep-wipe**: a band of color orbs, flowing on orber's
one-way conveyor, sweeps across the frame and washes the base from `from` to `to`
as it passes. It is *not* a translucent cross-fade and *not* a global occlusion
pulse — the orbs cover one perpendicular slice (the band) at a time, never the
whole frame at once.

- `t = 0`: the band is off the entry edge; the whole frame is `from`.
- `t → 1`: the band (a strip perpendicular to the flow) sweeps from the entry
  edge to the exit edge. **Ahead** of the band the base is still `from`;
  **behind** it the base is already `to`. The `from → to` boundary (the *seam*)
  always rides **inside** the band, so it is never directly visible — the orbs
  (carrying `from`'s palette) wash over it and leave `to` in their wake.
- `t = 1`: the band has left by the exit edge; the whole frame is `to`.

The base layer is a hard directional `from → to` step at the **wipe front**
`p(t)`, which travels off-frame at both ends along `--direction`
(`lr`/`rl`/`tb`/`bt`); reversing the direction reverses which edge `to` grows
from. The orbs tile a jittered grid whose cross-axis rows cover the band's full
width (gap-free) and whose flow-axis columns give the band depth, centered on the
front and riding a one-way drift (`--speed`); `--count` sets density and
`--orb-size` scales the disc radius and band thickness. The mechanism is pinned
by tests on both the CPU and GPU paths: t=0/t=1 endpoints, **monotone** growth of
the `to` region (the front never retreats), and **seam coverage** (the from/to
boundary is hidden under the band — no raw hard seam shows). Strict CPU↔GPU pixel
parity is *not* required (orber itself split over exactly that rasterizer
mismatch).

## Relationship to orber

`No.13 — orb-dissolve` reuses [orber](https://github.com/kako-jun/orber)'s orb
engine (`orber-core`: color clustering → orb placement) so it does not reinvent
orbs. orber is the *supplier*; ADDITIVE-13 is a *consumer*. orber itself stays a
single-input stylizer — no `--transition` mode is bolted onto it.
