//! No.13 — orb-dissolve (flagship).
//!
//! `from` shatters into a field of drifting orbs (color clusters lifted from the
//! image via [`orber-core`](orber_core)) which clear away to reveal `to` beneath.
//! The orb motion reuses orber's **one-way conveyor** model (see orber's
//! `docs/overview.md` "Motion model"): every orb drifts in a single direction,
//! wrapping fully off-screen over `[-r, 1+r]`, with a per-orb integer speed
//! multiplier and three-axis breathing.
//!
//! ## Renderer parity caveat
//!
//! Unlike No.0 crossfade, orb-dissolve does **not** assert strict pixel parity
//! between the CPU (tiny-skia, via orber's `render_static`) and GPU (WGSL) paths.
//! orber itself split over exactly this two-rasterizer mismatch; complex orb
//! drawing will not agree channel-for-channel. The tests here verify the
//! *mechanism* (t=0 ≈ from, t=1 ≈ to, orb pool non-empty) instead.

#[cfg(feature = "gpu")]
use image::RgbaImage;

#[cfg(feature = "gpu")]
use crate::transition::Transition;

#[cfg(feature = "gpu")]
use orber_core::cluster::{drop_dominant, extract_clusters, Centroid, Cluster};
#[cfg(feature = "gpu")]
use orber_core::orb::{render_static, OrbShape, RenderOptions};
#[cfg(feature = "gpu")]
use orber_core::style::SoftnessPreset;

/// WGSL production shader for [`OrbDissolve`].
#[cfg(feature = "gpu")]
pub const ORB_DISSOLVE_WGSL: &str = include_str!("orb_dissolve.wgsl");

/// Number of color clusters to extract from `from`. A handful more than orber's
/// typical palette so the dominant (background) drop still leaves a lively pool.
#[cfg(feature = "gpu")]
const CLUSTER_K: usize = 12;

/// Maximum orbs the WGSL fragment loop iterates (must match `orb_dissolve.wgsl`).
#[cfg(feature = "gpu")]
pub const MAX_ORBS: usize = 16;

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

/// **No.13 — orb-dissolve.** The flagship additive: `from` dissolves into a field
/// of drifting orbs (color clusters via orber-core) that thin out to reveal `to`.
pub struct OrbDissolve;

#[cfg(feature = "gpu")]
impl OrbDissolve {
    /// Extract the orb pool from `from`: cluster its colors and drop the dominant
    /// (background) one, exactly like orber's orb placement. Falls back to an
    /// empty pool if clustering fails (e.g. a 0-px image).
    pub fn orb_pool(from: &RgbaImage) -> Vec<Cluster> {
        let rgb = rgba_to_rgb(from);
        match extract_clusters(&rgb, CLUSTER_K) {
            Ok(clusters) => drop_dominant(&clusters),
            Err(_) => Vec::new(),
        }
    }

    /// Compute the live orb instances at time `t` from a static `pool`.
    ///
    /// Each orb is anchored at its **cluster centroid `(x, y)`** (orber's 2D
    /// placement) and drifts along the conveyor axis (left→right by default),
    /// wrapping over `[-r, 1+r]` so it never pops in/out on screen, at a per-orb
    /// integer speed multiplier, with breathing on radius. Only the flow axis
    /// (`x` for `lr`) gains the conveyor offset; the cross axis (`y`) keeps the
    /// orb's own centroid so the field scatters in 2D instead of collapsing onto
    /// a single horizontal band.
    ///
    /// Color-only k-means on a smooth gradient can give every cluster the same
    /// spatial centroid (each color stripe spans the full cross axis, so its mean
    /// lands at ≈0.5). To keep the field from stacking on one line in that
    /// degenerate case, the centroid `y` is combined with a small deterministic,
    /// index-derived cross-axis spread (same golden-ratio scheme as `phase`). For
    /// genuinely 2D-distributed images the centroid dominates; for collapsed
    /// gradients the spread still fans the orbs across the frame.
    ///
    /// The cross-fade *envelope* (`appear → drift → clear`) is keyed to global
    /// `t`: orbs are absent at `t = 0`, peak mid-clip, and fully gone at `t = 1`.
    pub fn orb_instances(pool: &[Cluster], t: f32) -> Vec<OrbInstance> {
        let t = t.clamp(0.0, 1.0);
        // Global appearance envelope: 0 at the ends, peak at the middle. A simple
        // raised sine keeps t=0≈from and t=1≈to (no orbs obscuring either end).
        let envelope = (std::f32::consts::PI * t).sin();

        pool.iter()
            .take(MAX_ORBS)
            .enumerate()
            .map(|(i, c)| {
                // Per-orb deterministic pseudo-params derived from the index, in
                // the spirit of orber's seed-derived phase/speed (no rng dep here).
                let phase = fract(0.1 + (i as f32) * 0.618_034);
                let speed = (1 + (i % 3)) as f32; // 1x / 2x / 3x
                                                  // Radius normalized by the short axis, from cluster weight
                                                  // (orber: radius ∝ sqrt(weight)). 0.25 is orber's base unit.
                let base_r = 0.25 * c.weight.max(0.0).sqrt();
                // Breathing on radius (±10%, orber's axis), looping once per clip.
                let breath = 1.0 + 0.10 * (std::f32::consts::TAU * (t + phase)).sin();
                let radius = (base_r * breath).max(0.0);

                // One-way conveyor along the flow axis (x): the orb's centroid x
                // is the anchor, and progress wraps over [-r, 1+r] so it never
                // pops in/out on screen.
                let span = 1.0 + 2.0 * radius;
                let raw = c.centroid.x.clamp(0.0, 1.0) + phase + speed * t;
                let prog = -radius + fract(raw) * span;

                // Cross axis (y): keep the orb's own centroid, plus a small
                // deterministic, index-derived spread so a degenerate gradient
                // (every cluster centroid ≈ 0.5) still fans out in 2D rather than
                // collapsing onto one horizontal band. Golden-ratio low-discrepancy
                // sequence centered on 0 gives an even, repeat-free cross spread.
                let scatter = fract(0.37 + (i as f32) * 0.381_966) - 0.5; // [-0.5, 0.5)
                let cy = (c.centroid.y + 0.7 * scatter).clamp(0.0, 1.0);

                let [r, g, b] = c.color;
                OrbInstance {
                    pos: [prog, cy],
                    radius,
                    alpha: envelope,
                    color: [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0],
                }
            })
            .collect()
    }

    /// Build the GPU orb array for `from` at time `t`, ready to hand to
    /// [`crate::gpu::GpuRenderer::render_orbs`].
    pub fn gpu_orbs(pool: &[Cluster], t: f32) -> Vec<crate::gpu::GpuOrb> {
        Self::orb_instances(pool, t)
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
        "`from` shatters into drifting orbs (via orber-core) and clears to reveal `to`."
    }

    fn render_cpu(&self, from: &RgbaImage, to: &RgbaImage, t: f32) -> RgbaImage {
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

        // Layer 1: `to` as the opaque background.
        let mut out = to.clone();

        // Layer 2: `from` fading out over the clip (alpha = 1 - t).
        let from_alpha = 1.0 - t;
        for (o, f) in out.pixels_mut().zip(from.pixels()) {
            for c in 0..3 {
                let v = f.0[c] as f32 * from_alpha + o.0[c] as f32 * (1.0 - from_alpha);
                o.0[c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }

        // Layer 3: drifting orbs, drawn by orber's tiny-skia `render_static` onto
        // a transparent canvas, then composited with the per-frame envelope alpha.
        let pool = Self::orb_pool(from);
        let instances = Self::orb_instances(&pool, t);
        if !instances.is_empty() {
            // Rebuild clusters at the live positions for render_static. weight is
            // back-derived from the instance radius so orber draws the same size:
            // radius = 0.25 * sqrt(weight)  =>  weight = (radius / 0.25)^2.
            let live: Vec<Cluster> = instances
                .iter()
                .map(|o| Cluster {
                    color: [
                        (o.color[0] * 255.0).round() as u8,
                        (o.color[1] * 255.0).round() as u8,
                        (o.color[2] * 255.0).round() as u8,
                    ],
                    centroid: Centroid {
                        x: o.pos[0].clamp(0.0, 1.0),
                        y: o.pos[1].clamp(0.0, 1.0),
                    },
                    weight: (o.radius / 0.25).powi(2),
                })
                .collect();

            let opts = RenderOptions {
                width: w,
                height: h,
                orb_size: 1.0,
                blur: 0.6,
                saturation: 1.0,
                background: [0, 0, 0, 0], // transparent so we can composite
                shape: OrbShape::Circle,
                softness: SoftnessPreset::Mid,
            };
            let orb_layer = render_static(&live, &opts);

            // Composite the orb layer with the global appearance envelope.
            let envelope = instances[0].alpha; // same envelope for all orbs
            for (o, orb) in out.pixels_mut().zip(orb_layer.pixels()) {
                let a = (orb.0[3] as f32 / 255.0) * envelope;
                if a <= 0.0 {
                    continue;
                }
                for c in 0..3 {
                    let v = orb.0[c] as f32 * a + o.0[c] as f32 * (1.0 - a);
                    o.0[c] = v.round().clamp(0.0, 255.0) as u8;
                }
            }
        }

        // Opaque output (baked mode); alpha overlay lands separately.
        for px in out.pixels_mut() {
            px.0[3] = 255;
        }
        out
    }

    fn shader_wgsl(&self) -> &'static str {
        ORB_DISSOLVE_WGSL
    }
}

/// CPU-only stub when the `gpu` feature (and thus orber-core) is off. orb-dissolve
/// is a GPU-feature additive — without orber-core there is no orb engine — so it
/// is simply not registered in that build (see `transition::all`). This bare type
/// keeps the module compiling for documentation purposes.
#[cfg(not(feature = "gpu"))]
impl OrbDissolve {}

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

#[cfg(all(test, feature = "gpu"))]
mod tests {
    use super::*;
    use image::Rgba;

    /// A photo-ish gradient with a few distinct color regions so clustering finds
    /// real orbs (not a single flat color).
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
        // At t=0 the orb envelope is 0 and from fully opaque, so the frame is from.
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
        // At t=1 the orb envelope is 0 and from fully faded, so the frame is to.
        assert!(d_to < 1.0, "t=1 should be (almost) exactly `to`");
        assert!(
            d_to < d_from,
            "t=1 must be much closer to `to` than to `from`"
        );
    }

    #[test]
    fn midpoint_shows_orbs() {
        // At the envelope peak the orb layer must perturb the frame away from a
        // plain crossfade of the two endpoints — i.e. orbs are actually drawn.
        let from = sample_image(96, 96, false);
        let to = sample_image(96, 96, true);
        let od = OrbDissolve;
        let frame = od.render_cpu(&from, &to, 0.5);
        // Plain 50/50 crossfade for reference.
        let mut blend = RgbaImage::new(96, 96);
        for (o, (f, g)) in blend.pixels_mut().zip(from.pixels().zip(to.pixels())) {
            for c in 0..3 {
                o.0[c] = ((f.0[c] as u16 + g.0[c] as u16) / 2) as u8;
            }
            o.0[3] = 255;
        }
        let d = mean_rgb_diff(&frame, &blend);
        eprintln!("t=0.5: mean diff from plain crossfade = {d:.2}");
        assert!(d > 0.5, "orbs should visibly perturb the midpoint frame");
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
        let pool = OrbDissolve::orb_pool(&from);
        let a = OrbDissolve::orb_instances(&pool, 0.2);
        let b = OrbDissolve::orb_instances(&pool, 0.7);
        assert!(!a.is_empty() && !b.is_empty());
        // At least one orb moved along x between the two times.
        let moved = a
            .iter()
            .zip(b.iter())
            .any(|(p, q)| (p.pos[0] - q.pos[0]).abs() > 1e-3);
        assert!(moved, "orbs must drift along the conveyor axis over time");
    }
}
