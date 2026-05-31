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
| No.13       | `orb-dissolve` | planned     | `from` shatters into drifting orbs (via orber-core) and clears to reveal `to`. Flagship. |

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

The CPU renderer in `additive-13-core` exists as the **reference / parity oracle**
the wgpu path is verified against, and to give the web GUI a working (if slower)
preview before the wgpu path lands.

## Output modes

- **Baked** — composite `from → to` internally and emit an opaque `mp4` / `webm`.
  Simplest; this is what validates the look first.
- **Alpha overlay** — emit only the dissolving layer with an alpha channel, so
  name-name (or any compositor) can play it over live content. The browser path
  reuses orber's hard-won approach: PNG frames muxed into a QuickTime `.mov`
  container (RGBA 32-bit lossless, NLE-compatible) rather than fighting
  ffmpeg.wasm's vp9-alpha — see orber's `web/src/lib/movMuxer.ts`.

## Relationship to orber

`No.13 — orb-dissolve` reuses [orber](https://github.com/kako-jun/orber)'s orb
engine (`orber-core`: color clustering → orb placement) so it does not reinvent
orbs. orber is the *supplier*; ADDITIVE-13 is a *consumer*. orber itself stays a
single-input stylizer — no `--transition` mode is bolted onto it.
