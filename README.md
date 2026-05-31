# ADDITIVE-13

A series of image **transitions** — "additives" — that dissolve one picture into
another. Each additive is a pure function of normalized time `t`:

```
(from image, to image, t in 0..1)  ->  RGBA frame
```

The flagship, **No.13 — orb-dissolve**, breaks the *from* image into slowly
drifting orbs that fade away, revealing the *to* image beneath. (The name nods to
パトレイバー's 廃棄物13号 and to the way food additives are catalogued by number.)

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

The reference CPU renderer in `additive-13-core` is the parity oracle the wgpu
path is checked against — not the production path.

## Usage

The product output is **video** — a transition is something you play, not a still.

```bash
# A frame sequence over the whole transition (feeds the video encoder)
additive-13 --from a.jpg --to b.jpg --frames 48 --out-dir frames/

# List available additives
additive-13 --list

# Debug peek: render one frame at a given t (for eyeballing / parity tests, not a product feature)
additive-13 --from a.jpg --to b.jpg --output peek.png --t 0.5
```

Video muxing (opaque `mp4` and transparent `mov` for overlay compositing) lands
with the wgpu renderer; see the roadmap. Single-image *stylizing* is intentionally
out of scope — that is [orber](https://github.com/kako-jun/orber)'s job. ADDITIVE-13
is strictly two-input transitions.

## Layout

```
crates/core   additive-13-core — the Transition contract + reference renderer (wasm-buildable, no I/O)
crates/cli    additive-13      — the command-line tool (image I/O, encoding)
crates/wasm   additive-13-wasm — browser bindings
web/          Astro + Solid web app (planned)
```

## License

MIT © 2026 kako-jun
