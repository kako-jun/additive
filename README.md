# ADDITIVE-13

A series of image **transitions** — "additives" — that dissolve one picture into
another. Each additive is a pure function of normalized time `t`:

```
(from image, to image, t in 0..1)  ->  RGBA frame
```

The flagship, **No.13 — orb-dissolve**, sweeps a band of *from*-colored orbs
across the frame in one direction (right→left by default): everything the band has
already passed is *to*, everything ahead is still *from*, and the from→to seam is
always hidden inside the moving band of orbs (`--count` / `--speed` / `--direction`
/ `--orb-size` tune the sweep). (The name nods to パトレイバー's 廃棄物13号 and to the
way food additives are catalogued by number.)

> **Status:** prototype scaffold. The reference CPU renderer works end-to-end
> (`No.0 — crossfade`); the production wgpu renderer and `No.13 — orb-dissolve`
> are in progress. See `docs/roadmap.md`.

## Why two homes

ADDITIVE-13 is built to live in two places at once, like a transition you can
both *use* and *ship*:

- **Inside [name-name](https://github.com/kako-jun/name-name)** — as background /
  scene-switching effects in the visual-novel engine.
- **Standalone** — a CLI and a web app, the same way [orber](https://github.com/kako-jun/orber)
  is both a library and a product.

## One renderer, identical output

The hard constraint that shapes the architecture: **the CLI and the browser must
produce the same result**, and **the browser must preview in real time**. Only one
approach satisfies both — a single **wgpu** renderer written once in Rust + WGSL,
compiled to native (CLI) and to wasm/WebGPU (browser). No separate WebGL shader,
no separate CPU path diverging from the GPU one. (See `docs/overview.md` for why
raw WebGL or a CPU-only wasm core each fail one half of the constraint.)

The reference CPU renderer in `additive-core` is the parity oracle the wgpu
path is checked against — not the production path.

## Usage

The product output is **video** — a transition is something you play, not a still.

```bash
# Baked video — the product output. The extension picks the codec:
#   .mp4  -> H.264 (libx264, yuv420p)   .webm -> VP9 (libvpx-vp9, yuv420p)
additive --from a.jpg --to b.jpg --transition orb-dissolve --output out.mp4
additive --from a.jpg --to b.jpg --transition crossfade   --output out.webm

# Video length / frame rate (defaults: 2500ms, 30fps)
additive --from a.jpg --to b.jpg --output out.mp4 --duration-ms 2000 --fps 30

# A raw frame sequence over the whole transition (no encoder)
additive --from a.jpg --to b.jpg --frames 48 --out-dir frames/

# List available additives
additive --list

# Debug peek: render one frame at a given t (for eyeballing / parity tests, not a product feature)
additive --from a.jpg --to b.jpg --output peek.png --t 0.5

# No.13 orb-dissolve curtain knobs (ignored by other transitions)
additive --from a.jpg --to b.jpg --transition orb-dissolve --output out.mp4 \
  --count 100 --orb-size 1.5 --direction tb --speed 1.5
```

Baked video needs **ffmpeg** on your `PATH` (the CLI shells out to it). Video
output rounds odd source dimensions *down* to even (e.g. 37×23 → 36×22), since
H.264/VP9 `yuv420p` requires even width and height; `--duration-ms` is capped at
600000 (10 minutes). Opaque
`mp4` / `webm` work today; transparent `mov` for overlay compositing (`--alpha`)
is a follow-up — see the roadmap. Single-image *stylizing* is intentionally out of
scope — that is [orber](https://github.com/kako-jun/orber)'s job. ADDITIVE-13 is
strictly two-input transitions.

## Layout

```
crates/core   additive-core — the Additive contract (Transition + Generator) + reference renderer (wasm-buildable, no I/O)
crates/cli    additive      — the command-line tool (image I/O, encoding)
crates/wasm   additive-wasm — browser bindings
web/          Astro + Solid web app (planned)
```

## License

MIT © 2026 kako-jun
