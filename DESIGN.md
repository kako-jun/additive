# DESIGN.md

ADDITIVE-13 — Design System

The web GUI follows orber's visual language: **black-canvas gothic with
glass-only controls**, no hue accent, Space Grotesk typography, state expressed
by opacity steps rather than color. The generated transition supplies all the
color; the chrome stays monochrome.

This file will carry the full token table, component specs, and i18n rules once
the web GUI is implemented (issue #5). Until then, treat orber's `DESIGN.md` as
the reference and keep ADDITIVE-13 visually continuous with it — the two are
sibling products in the same family.

## Studio shape (planned, #5)

- Two drop areas (`from`, `to`) instead of orber's single input.
- An additive picker (the catalogue: No.0, No.13, …).
- A scrubber over `t` with a live wgpu preview.
- Export: opaque `mp4` (baked) and transparent `.mov` (overlay), reusing orber's
  `movMuxer.ts` for the alpha path.
