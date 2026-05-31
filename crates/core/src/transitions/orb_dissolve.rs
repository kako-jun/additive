//! No.13 — orb-dissolve (flagship), **full-occlusion redefinition**.
//!
//! A curtain of color orbs (palette lifted from `from` via
//! [`orber-core`](orber_core) clustering) **completely covers the screen** at the
//! transition's peak, and the `from → to` base layer is swapped underneath while
//! nothing of either image shows through. This makes No.13 a *wipe-style* dissolve
//! (cover, then swap) rather than a translucent cross-fade.
//!
//! ## Timeline
//!
//! - `t = 0`: pure `from` (no orbs / orbs at radius 0).
//! - `t → 0.5`: orbs grow and their opacity rises until, across the **occlusion
//!   plateau** around `t = 0.5`, the whole frame is opaque orbs (full occlusion).
//! - `t = 0.5`: the base layer **hard-swaps** `from → to` (with a ±0.05 micro
//!   cross-fade) — invisible because the orbs cover everything.
//! - `t → 1`: orbs shrink / fade and recede, revealing `to`.
//! - `t = 1`: pure `to`.
//!
//! ## Coverage model
//!
//! To guarantee a gap-free curtain, orbs are placed on a **jittered grid** sized
//! from `--count` (orber's color clusters seed the *palette*; the grid guarantees
//! *coverage*). Each cell holds one orb whose color is sampled from `from` at the
//! orb's position. The orb radius is driven by an **occlusion envelope** (0 at the
//! ends, a wide plateau at its max in the middle) scaled so neighbouring orbs
//! overlap by a healthy margin (`COVER_OVERLAP`) — at the plateau the discs are
//! big enough, and opaque enough, to leave no hole. The whole field rides orber's
//! one-way conveyor along `--direction`, so the curtain also drifts.
//!
//! ## Renderer parity caveat
//!
//! Unlike No.0 crossfade, orb-dissolve does **not** assert strict pixel parity
//! between the CPU (tiny-skia) and GPU (WGSL) paths — orber itself split over
//! exactly that two-rasterizer mismatch. The tests assert the *mechanism*: full
//! occlusion (the rendered frame is independent of the base image at the peak),
//! the t=0 / t=1 endpoints, and that the orb field actually covers.

#[cfg(feature = "gpu")]
use image::RgbaImage;

#[cfg(feature = "gpu")]
use crate::transition::Transition;

#[cfg(feature = "gpu")]
use orber_core::cluster::{drop_dominant, extract_clusters, Cluster};

/// WGSL production shader for [`OrbDissolve`].
#[cfg(feature = "gpu")]
pub const ORB_DISSOLVE_WGSL: &str = include_str!("orb_dissolve.wgsl");

/// Number of color clusters to extract from `from` for the orb *palette*. The
/// grid provides coverage; clustering only provides representative colors as a
/// fallback when per-pixel sampling is unavailable.
#[cfg(feature = "gpu")]
const CLUSTER_K: usize = 12;

/// Maximum orbs the WGSL fragment loop iterates (must match `MAX_ORBS` in
/// `orb_dissolve.wgsl` and `gpu::MAX_ORBS`). Raised from 16 → 128 so a dense
/// enough grid can fully cover the frame.
#[cfg(feature = "gpu")]
pub const MAX_ORBS: usize = 128;

/// Default orb count (≈ an 8×8 grid): dense enough for gap-free coverage at the
/// default `--orb-size`.
#[cfg(feature = "gpu")]
pub const DEFAULT_COUNT: u32 = 64;

/// Default conveyor speed multiplier.
#[cfg(feature = "gpu")]
pub const DEFAULT_SPEED: f32 = 1.0;

/// Default orb-size multiplier.
#[cfg(feature = "gpu")]
pub const DEFAULT_ORB_SIZE: f32 = 1.0;

/// How much neighbouring orbs overlap at the plateau, as a multiple of the grid
/// cell's half-diagonal. > 1 guarantees the soft discs' opaque cores meet with no
/// gap even after the soft rim is discounted.
#[cfg(feature = "gpu")]
const COVER_OVERLAP: f32 = 1.85;

/// Conveyor drift direction (the axis the orb curtain travels along).
///
/// The curtain always covers the *whole* frame; `direction` only sets which way
/// the field slides while it does so (and which way it clears).
#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbDirection {
    /// Left → right.
    Lr,
    /// Right → left.
    Rl,
    /// Top → bottom.
    Tb,
    /// Bottom → top.
    Bt,
}

#[cfg(feature = "gpu")]
impl OrbDirection {
    /// Parse the CLI spelling (`lr` / `rl` / `tb` / `bt`).
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "lr" => Some(Self::Lr),
            "rl" => Some(Self::Rl),
            "tb" => Some(Self::Tb),
            "bt" => Some(Self::Bt),
            _ => None,
        }
    }

    /// `true` when the conveyor flows along the horizontal (x) axis.
    fn is_horizontal(self) -> bool {
        matches!(self, Self::Lr | Self::Rl)
    }

    /// `true` when the drift runs in the negative direction of its axis.
    fn is_negative(self) -> bool {
        matches!(self, Self::Rl | Self::Bt)
    }
}

/// Tunable knobs for the orb curtain, surfaced on the CLI as `--count` /
/// `--speed` / `--direction` / `--orb-size`. Defaults are chosen so full
/// occlusion is reached around `t = 0.5` out of the box.
#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug)]
pub struct OrbConfig {
    /// Number of orbs (clamped to `[1, MAX_ORBS]`). Drives the coverage grid.
    pub count: u32,
    /// Conveyor speed multiplier (how fast the curtain drifts).
    pub speed: f32,
    /// Drift direction.
    pub direction: OrbDirection,
    /// Orb-size multiplier on top of the coverage-derived radius.
    pub orb_size: f32,
}

#[cfg(feature = "gpu")]
impl Default for OrbConfig {
    fn default() -> Self {
        Self {
            count: DEFAULT_COUNT,
            speed: DEFAULT_SPEED,
            direction: OrbDirection::Lr,
            orb_size: DEFAULT_ORB_SIZE,
        }
    }
}

/// Per-orb data uploaded to the GPU (and used to drive the CPU path too).
///
/// `pos` is the orb center in normalized `[0,1]^2` UV, `radius` is normalized by
/// the shorter axis, `color` is straight sRGB `[0,1]`, `alpha` the orb's current
/// envelope opacity.
#[cfg(feature = "gpu")]
#[derive(Clone, Copy)]
pub struct OrbInstance {
    pub pos: [f32; 2],
    pub radius: f32,
    pub alpha: f32,
    pub color: [f32; 3],
}

/// **No.13 — orb-dissolve.** A curtain of color orbs covers the frame completely
/// at the peak, then the `from → to` base is swapped underneath (full-occlusion
/// wipe), and the curtain clears to reveal `to`.
pub struct OrbDissolve;

#[cfg(feature = "gpu")]
impl OrbDissolve {
    /// Extract a color *palette* from `from` (clusters minus the dominant
    /// background), used as a fallback when per-pixel sampling is unavailable.
    /// Returns an empty pool on a degenerate image.
    pub fn orb_pool(from: &RgbaImage) -> Vec<Cluster> {
        let rgb = rgba_to_rgb(from);
        match extract_clusters(&rgb, CLUSTER_K) {
            Ok(clusters) => drop_dominant(&clusters),
            Err(_) => Vec::new(),
        }
    }

    /// The base-layer swap fraction at time `t`: the weight of `from` (so `to`
    /// weight is `1 - this`). Hard-swaps at `t = 0.5` with a ±[`SWAP_HALF`] micro
    /// cross-fade, fully hidden by the orb curtain at the plateau.
    pub fn base_from_weight(t: f32) -> f32 {
        const SWAP_HALF: f32 = 0.05;
        let t = t.clamp(0.0, 1.0);
        // from weight: 1 below 0.5-half, 0 above 0.5+half, linear across the band.
        let lo = 0.5 - SWAP_HALF;
        let hi = 0.5 + SWAP_HALF;
        if t <= lo {
            1.0
        } else if t >= hi {
            0.0
        } else {
            1.0 - (t - lo) / (hi - lo)
        }
    }

    /// Occlusion envelope in `[0,1]`: 0 at the timeline ends, a flat plateau at
    /// `1.0` across the middle. Drives both orb radius (coverage) and opacity, so
    /// the curtain is simultaneously largest and fully opaque while the base swaps.
    pub fn occlusion_envelope(t: f32) -> f32 {
        // Plateau spans [PLATEAU_LO, PLATEAU_HI]; smoothstep ramps on either side.
        const PLATEAU_LO: f32 = 0.40;
        const PLATEAU_HI: f32 = 0.60;
        let t = t.clamp(0.0, 1.0);
        if t < PLATEAU_LO {
            smoothstep(0.0, PLATEAU_LO, t)
        } else if t > PLATEAU_HI {
            1.0 - smoothstep(PLATEAU_HI, 1.0, t)
        } else {
            1.0
        }
    }

    /// Compute the live orb instances at time `t` for the given `cfg`, sampling
    /// each orb's color from `from`.
    ///
    /// Orbs sit on a jittered near-square grid that tiles `[0,1]^2`, so the field
    /// covers the whole frame. Their radius follows [`occlusion_envelope`] scaled
    /// to overlap neighbours by [`COVER_OVERLAP`], and their opacity is the same
    /// envelope (opaque across the plateau ⇒ full occlusion). The whole grid rides
    /// orber's one-way conveyor along `cfg.direction` at `cfg.speed`.
    pub fn orb_instances(from: &RgbaImage, cfg: &OrbConfig, t: f32) -> Vec<OrbInstance> {
        let t = t.clamp(0.0, 1.0);
        let count = cfg.count.clamp(1, MAX_ORBS as u32) as usize;
        let envelope = Self::occlusion_envelope(t);

        // Grid dimensions: a near-square that holds `count` cells.
        let cols = (count as f32).sqrt().ceil().max(1.0) as usize;
        let rows = count.div_ceil(cols).max(1);

        // Coverage radius: a cell's half-diagonal × overlap × user size, in the
        // shorter-axis-normalized units the shader/CPU draw in. Cells are sized in
        // normalized [0,1] UV; the half-diagonal is sqrt((1/2cols)^2+(1/2rows)^2).
        let cell_hw = 0.5 / cols as f32;
        let cell_hh = 0.5 / rows as f32;
        let half_diag = (cell_hw * cell_hw + cell_hh * cell_hh).sqrt();
        let cover_radius = half_diag * COVER_OVERLAP * cfg.orb_size.max(0.0);
        let radius = cover_radius * envelope;

        let (iw, ih) = from.dimensions();
        let pool = Self::orb_pool(from);

        let mut out = Vec::with_capacity(count);
        for idx in 0..count {
            let gx = idx % cols;
            let gy = idx / cols;

            // Cell-center home position in [0,1]^2.
            let hx = (gx as f32 + 0.5) / cols as f32;
            let hy = (gy as f32 + 0.5) / rows as f32;

            // Deterministic per-orb jitter (golden-ratio low-discrepancy), kept
            // small so cells stay anchored to their slot (coverage) yet the grid
            // doesn't read as a rigid lattice. Jitter is a fraction of a cell.
            let jx = (fract(0.137 + idx as f32 * 0.618_034) - 0.5) * cell_hw * 0.8;
            let jy = (fract(0.731 + idx as f32 * 0.381_966) - 0.5) * cell_hh * 0.8;
            let home_x = (hx + jx).clamp(0.0, 1.0);
            let home_y = (hy + jy).clamp(0.0, 1.0);

            // One-way conveyor along the chosen axis. The whole curtain shares a
            // *single* drift offset (so the grid keeps its even spacing and tiles
            // gap-free no matter where it has slid to), wrapped modulo one cell
            // along the drift axis. The perpendicular axis stays anchored to the
            // home grid, so coverage is preserved at the plateau while the field
            // still visibly travels.
            let dir = &cfg.direction;
            let mut offset = cfg.speed * t;
            if dir.is_negative() {
                offset = -offset;
            }
            let (px, py) = if dir.is_horizontal() {
                // Slide along x by `offset` cells, wrapping by one cell width.
                let cell_w = 1.0 / cols as f32;
                let prog = home_x + fract(offset) * cell_w;
                let prog = fract(prog); // keep inside [0,1) so the row stays covered
                (prog, home_y)
            } else {
                let cell_h = 1.0 / rows as f32;
                let prog = home_y + fract(offset) * cell_h;
                let prog = fract(prog);
                (home_x, prog)
            };

            // Color: sample `from` at the orb's home cell (so the curtain carries
            // from's palette region-by-region). Fall back to a cluster color, then
            // mid-grey, on a degenerate image.
            let color = sample_color(from, iw, ih, home_x, home_y, &pool, idx);

            out.push(OrbInstance {
                pos: [px, py],
                radius,
                alpha: envelope,
                color,
            });
        }
        out
    }

    /// Build the GPU orb array for `from` at time `t`, ready to hand to
    /// [`crate::gpu::GpuRenderer::render_orbs`].
    pub fn gpu_orbs(from: &RgbaImage, cfg: &OrbConfig, t: f32) -> Vec<crate::gpu::GpuOrb> {
        Self::orb_instances(from, cfg, t)
            .into_iter()
            .map(|o| crate::gpu::GpuOrb {
                pos_radius_alpha: [o.pos[0], o.pos[1], o.radius, o.alpha],
                color: [o.color[0], o.color[1], o.color[2], 0.0],
            })
            .collect()
    }
}

#[cfg(feature = "gpu")]
impl Transition for OrbDissolve {
    fn designation(&self) -> &'static str {
        "No.13"
    }

    fn name(&self) -> &'static str {
        "orb-dissolve"
    }

    fn description(&self) -> &'static str {
        "`from` is covered by a curtain of orbs that fully occludes the frame, then swaps to `to`."
    }

    fn render_cpu(&self, from: &RgbaImage, to: &RgbaImage, t: f32) -> RgbaImage {
        self.render_cpu_cfg(from, to, t, &OrbConfig::default())
    }

    fn shader_wgsl(&self) -> &'static str {
        ORB_DISSOLVE_WGSL
    }
}

#[cfg(feature = "gpu")]
impl OrbDissolve {
    /// CPU reference render with explicit [`OrbConfig`] (the trait method uses the
    /// default config; the CLI threads the user's knobs here).
    ///
    /// Base layer is a hard `from → to` swap at `t = 0.5` (the orb curtain hides
    /// it). The orb curtain is drawn as opaque soft discs on top, matching the
    /// WGSL falloff so CPU and GPU read the same.
    pub fn render_cpu_cfg(
        &self,
        from: &RgbaImage,
        to: &RgbaImage,
        t: f32,
        cfg: &OrbConfig,
    ) -> RgbaImage {
        debug_assert_eq!(
            from.dimensions(),
            to.dimensions(),
            "from and to must share dimensions"
        );
        let t = t.clamp(0.0, 1.0);
        let (w, h) = from.dimensions();
        if w == 0 || h == 0 {
            return RgbaImage::new(w, h);
        }

        // Base layer: from→to hard swap (micro cross-fade across t≈0.5).
        let from_w = Self::base_from_weight(t);
        let mut out = RgbaImage::new(w, h);
        for (o, (f, g)) in out.pixels_mut().zip(from.pixels().zip(to.pixels())) {
            for c in 0..3 {
                let v = f.0[c] as f32 * from_w + g.0[c] as f32 * (1.0 - from_w);
                o.0[c] = v.round().clamp(0.0, 255.0) as u8;
            }
            o.0[3] = 255;
        }

        // Orb curtain. Drawn directly with the same soft-disc falloff as the WGSL
        // so the two renderers read the same (coverage is a geometric property, so
        // they agree on full occlusion even without bit parity).
        let instances = Self::orb_instances(from, cfg, t);
        let short = w.min(h) as f32;
        let aspect_x = w as f32 / short; // UV→isotropic scale (match shader space)
        let aspect_y = h as f32 / short;
        for (x, y, px) in out.enumerate_pixels_mut() {
            let ux = (x as f32 + 0.5) / w as f32;
            let uy = (y as f32 + 0.5) / h as f32;
            let mut r = px.0[0] as f32;
            let mut g = px.0[1] as f32;
            let mut b = px.0[2] as f32;
            for o in &instances {
                if o.radius <= 0.0 || o.alpha <= 0.0 {
                    continue;
                }
                // Toroidal (wrapped) distance in shorter-axis units: the orb field
                // tiles [0,1]^2 and the conveyor wraps, so an orb near an edge also
                // covers the opposite edge — no seam shows where the curtain wraps.
                // (Matches the WGSL aspect fix and its `wrap_delta`.)
                let dx = wrap_delta(ux - o.pos[0]) * aspect_x;
                let dy = wrap_delta(uy - o.pos[1]) * aspect_y;
                let d = (dx * dx + dy * dy).sqrt();
                let inner = o.radius * 0.7; // wide opaque core for gap-free coverage
                let falloff = 1.0 - smoothstep(inner, o.radius, d);
                let a = (falloff * o.alpha).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                r = o.color[0] * 255.0 * a + r * (1.0 - a);
                g = o.color[1] * 255.0 * a + g * (1.0 - a);
                b = o.color[2] * 255.0 * a + b * (1.0 - a);
            }
            px.0[0] = r.round().clamp(0.0, 255.0) as u8;
            px.0[1] = g.round().clamp(0.0, 255.0) as u8;
            px.0[2] = b.round().clamp(0.0, 255.0) as u8;
            px.0[3] = 255;
        }
        out
    }
}

/// CPU-only stub when the `gpu` feature (and thus orber-core) is off. orb-dissolve
/// is a GPU-feature additive — without orber-core there is no orb engine — so it
/// is simply not registered in that build (see `transition::all`). This bare type
/// keeps the module compiling for documentation purposes.
#[cfg(not(feature = "gpu"))]
impl OrbDissolve {}

/// Sample a straight-sRGB `[0,1]` color from `from` at normalized `(u, v)`,
/// falling back to a palette cluster (then mid-grey) on a degenerate image.
#[cfg(feature = "gpu")]
fn sample_color(
    from: &RgbaImage,
    iw: u32,
    ih: u32,
    u: f32,
    v: f32,
    pool: &[Cluster],
    idx: usize,
) -> [f32; 3] {
    if iw > 0 && ih > 0 {
        let x = ((u * iw as f32) as u32).min(iw - 1);
        let y = ((v * ih as f32) as u32).min(ih - 1);
        let p = from.get_pixel(x, y);
        return [
            p.0[0] as f32 / 255.0,
            p.0[1] as f32 / 255.0,
            p.0[2] as f32 / 255.0,
        ];
    }
    if !pool.is_empty() {
        let c = pool[idx % pool.len()].color;
        return [
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
        ];
    }
    [0.5, 0.5, 0.5]
}

/// Convert an `RgbaImage` to `RgbImage` (drop alpha) for orber's clustering.
#[cfg(feature = "gpu")]
fn rgba_to_rgb(img: &RgbaImage) -> image::RgbImage {
    let (w, h) = img.dimensions();
    let mut out = image::RgbImage::new(w, h);
    for (o, p) in out.pixels_mut().zip(img.pixels()) {
        o.0 = [p.0[0], p.0[1], p.0[2]];
    }
    out
}

/// Fractional part in `[0,1)`.
#[cfg(feature = "gpu")]
fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// Wrap a UV-space delta into `[-0.5, 0.5)` — the shortest signed distance on a
/// unit torus. Used so an orb near one edge of the wrapping curtain still covers
/// the opposite edge (no seam at the conveyor wrap).
#[cfg(feature = "gpu")]
fn wrap_delta(d: f32) -> f32 {
    d - (d + 0.5).floor()
}

/// GLSL-style `smoothstep(edge0, edge1, x)`.
#[cfg(feature = "gpu")]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge0 == edge1 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let s = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

#[cfg(all(test, feature = "gpu"))]
mod tests {
    use super::*;
    use image::Rgba;

    /// A photo-ish gradient with a few distinct color regions so the curtain
    /// carries real color variety.
    fn sample_image(w: u32, h: u32, swap: bool) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let (r, g, b) = if swap {
                ((fy * 255.0) as u8, (fx * 200.0) as u8, 120)
            } else {
                ((fx * 255.0) as u8, 80, (fy * 255.0) as u8)
            };
            *px = Rgba([r, g, b, 255]);
        }
        img
    }

    /// Mean absolute per-channel (RGB) difference between two images.
    fn mean_rgb_diff(a: &RgbaImage, b: &RgbaImage) -> f32 {
        assert_eq!(a.dimensions(), b.dimensions());
        let mut sum = 0u64;
        let mut n = 0u64;
        for (ap, bp) in a.pixels().zip(b.pixels()) {
            for c in 0..3 {
                sum += ap.0[c].abs_diff(bp.0[c]) as u64;
                n += 1;
            }
        }
        sum as f32 / n as f32
    }

    /// Max absolute per-channel (RGB) difference — proves *every* pixel agrees.
    fn max_rgb_diff(a: &RgbaImage, b: &RgbaImage) -> u8 {
        assert_eq!(a.dimensions(), b.dimensions());
        let mut m = 0u8;
        for (ap, bp) in a.pixels().zip(b.pixels()) {
            for c in 0..3 {
                m = m.max(ap.0[c].abs_diff(bp.0[c]));
            }
        }
        m
    }

    #[test]
    fn orb_pool_is_non_empty() {
        let from = sample_image(96, 96, false);
        let pool = OrbDissolve::orb_pool(&from);
        assert!(
            !pool.is_empty(),
            "clustering should yield a non-empty orb pool"
        );
        eprintln!("orb pool size: {}", pool.len());
    }

    #[test]
    fn t0_is_close_to_from() {
        let from = sample_image(96, 96, false);
        let to = sample_image(96, 96, true);
        let od = OrbDissolve;
        let frame = od.render_cpu(&from, &to, 0.0);
        let d_from = mean_rgb_diff(&frame, &from);
        let d_to = mean_rgb_diff(&frame, &to);
        eprintln!("t=0: mean diff to from={d_from:.2}, to to={d_to:.2}");
        // At t=0 the envelope is 0 (no orbs) and the base is fully `from`.
        assert!(d_from < 1.0, "t=0 should be (almost) exactly `from`");
        assert!(
            d_from < d_to,
            "t=0 must be much closer to `from` than to `to`"
        );
    }

    #[test]
    fn t1_is_close_to_to() {
        let from = sample_image(96, 96, false);
        let to = sample_image(96, 96, true);
        let od = OrbDissolve;
        let frame = od.render_cpu(&from, &to, 1.0);
        let d_from = mean_rgb_diff(&frame, &from);
        let d_to = mean_rgb_diff(&frame, &to);
        eprintln!("t=1: mean diff to from={d_from:.2}, to to={d_to:.2}");
        // At t=1 the envelope is 0 (no orbs) and the base is fully `to`.
        assert!(d_to < 1.0, "t=1 should be (almost) exactly `to`");
        assert!(
            d_to < d_from,
            "t=1 must be much closer to `to` than to `from`"
        );
    }

    /// **Core test: full occlusion.** At the peak the rendered frame must be
    /// *independent of the base image* — rendering with the base forced to `from`
    /// vs forced to `to` must produce (almost) identical output, because the orb
    /// curtain hides the base completely.
    #[test]
    fn peak_full_occlusion_base_independent() {
        let from = sample_image(128, 128, false);
        let to = sample_image(128, 128, true);
        let cfg = OrbConfig::default();
        let od = OrbDissolve;

        // Use the same orbs (sampled from `from`) but swap the *base*: render with
        // base=from-only vs base=to-only by handing render the two images as both
        // slots. The curtain orbs are identical (colors sampled from `from`), so
        // any residual difference is leaked base — which must be ~0.
        let with_from_base = od.render_cpu_cfg(&from, &from, 0.5, &cfg);
        let with_to_base = od.render_cpu_cfg(&from, &to, 0.5, &cfg);

        let d = mean_rgb_diff(&with_from_base, &with_to_base);
        let m = max_rgb_diff(&with_from_base, &with_to_base);
        eprintln!("t=0.5 full-occlusion: mean base-swap diff={d:.4}, max={m}");
        assert!(
            d < 0.5,
            "at the peak the base must be fully occluded (mean diff {d} too high)"
        );
        assert!(
            m <= 2,
            "at the peak no pixel should leak the base (max diff {m} too high)"
        );
    }

    /// Sweep t and report which range is fully occluded (base-independent).
    #[test]
    fn occlusion_plateau_spans_midpoint() {
        let from = sample_image(96, 96, false);
        let to = sample_image(96, 96, true);
        let cfg = OrbConfig::default();
        let od = OrbDissolve;
        let mut occluded = Vec::new();
        for k in 0..=20 {
            let t = k as f32 / 20.0;
            let a = od.render_cpu_cfg(&from, &from, t, &cfg);
            let b = od.render_cpu_cfg(&from, &to, t, &cfg);
            let d = mean_rgb_diff(&a, &b);
            if d < 0.5 {
                occluded.push(t);
            }
            eprintln!("t={t:.2}: base-swap diff={d:.4}");
        }
        eprintln!("fully occluded ts: {occluded:?}");
        assert!(
            occluded.contains(&0.5),
            "t=0.5 must be fully occluded; occluded set = {occluded:?}"
        );
    }

    #[test]
    fn empty_image_yields_empty() {
        let from = RgbaImage::new(0, 0);
        let to = RgbaImage::new(0, 0);
        let od = OrbDissolve;
        let frame = od.render_cpu(&from, &to, 0.5);
        assert_eq!(frame.dimensions(), (0, 0));
    }

    #[test]
    fn orb_instances_drift_with_t() {
        let from = sample_image(96, 96, false);
        let cfg = OrbConfig::default();
        let a = OrbDissolve::orb_instances(&from, &cfg, 0.35);
        let b = OrbDissolve::orb_instances(&from, &cfg, 0.65);
        assert!(!a.is_empty() && !b.is_empty());
        // At least one orb moved along the conveyor axis between the two times.
        let moved = a
            .iter()
            .zip(b.iter())
            .any(|(p, q)| (p.pos[0] - q.pos[0]).abs() > 1e-3 || (p.pos[1] - q.pos[1]).abs() > 1e-3);
        assert!(moved, "orbs must drift along the conveyor axis over time");
    }

    #[test]
    fn count_controls_orb_number() {
        let from = sample_image(96, 96, false);
        let mut cfg = OrbConfig {
            count: 9,
            ..Default::default()
        };
        let nine = OrbDissolve::orb_instances(&from, &cfg, 0.5);
        assert_eq!(nine.len(), 9, "--count must drive the orb count");
        cfg.count = 200; // above MAX_ORBS
        let capped = OrbDissolve::orb_instances(&from, &cfg, 0.5);
        assert_eq!(capped.len(), MAX_ORBS, "--count is capped at MAX_ORBS");
    }

    #[test]
    fn orb_size_scales_radius() {
        let from = sample_image(96, 96, false);
        let small = OrbConfig {
            orb_size: 1.0,
            ..Default::default()
        };
        let big = OrbConfig {
            orb_size: 2.0,
            ..Default::default()
        };
        let rs = OrbDissolve::orb_instances(&from, &small, 0.5)[0].radius;
        let rb = OrbDissolve::orb_instances(&from, &big, 0.5)[0].radius;
        assert!(
            rb > rs * 1.5,
            "--orb-size must scale radius (got {rs}, {rb})"
        );
    }

    #[test]
    fn direction_picks_axis() {
        let from = sample_image(96, 96, false);
        let lr = OrbConfig {
            direction: OrbDirection::Lr,
            ..Default::default()
        };
        let tb = OrbConfig {
            direction: OrbDirection::Tb,
            ..Default::default()
        };
        // Between two times, lr moves x, tb moves y (for the same orb).
        let lr0 = OrbDissolve::orb_instances(&from, &lr, 0.35);
        let lr1 = OrbDissolve::orb_instances(&from, &lr, 0.65);
        let tb0 = OrbDissolve::orb_instances(&from, &tb, 0.35);
        let tb1 = OrbDissolve::orb_instances(&from, &tb, 0.65);
        let lr_dx: f32 = (lr0[0].pos[0] - lr1[0].pos[0]).abs();
        let tb_dy: f32 = (tb0[0].pos[1] - tb1[0].pos[1]).abs();
        assert!(lr_dx > 1e-3, "lr must drift along x");
        assert!(tb_dy > 1e-3, "tb must drift along y");
    }

    #[test]
    fn base_swap_is_hard() {
        // Below the swap band the base is all `from`; above it, all `to`.
        assert_eq!(OrbDissolve::base_from_weight(0.0), 1.0);
        assert_eq!(OrbDissolve::base_from_weight(0.40), 1.0);
        assert_eq!(OrbDissolve::base_from_weight(0.60), 0.0);
        assert_eq!(OrbDissolve::base_from_weight(1.0), 0.0);
        assert!((OrbDissolve::base_from_weight(0.5) - 0.5).abs() < 1e-6);
    }
}
