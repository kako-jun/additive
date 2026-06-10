//! No.14 — aqua-dissolve, **watercolor seam dissolve (にじみ)**.
//!
//! Like No.13 orb-dissolve this is a directional `from → to` sweep — the wipe
//! front travels along the flow axis, `from` ahead of it and `to` behind it. But
//! where orb-dissolve *hides* the seam under a band of opaque orbs, aqua-dissolve
//! **dissolves the seam itself**: the from/to boundary is run through the shared
//! aquarelle spiral bleed (`aquarelle::AQUA_BLEED_WGSL` / `aqua_blurred_coverage_cpu`)
//! so the boundary wicks like wet pigment on paper instead of reading as a clean
//! wipe line. A faint saturated tint (`aqua_character` `halo`) is added at the wet
//! rim, like pigment pooling where the water front sits.
//!
//! ## Timeline
//!
//! - `t = 0`: the front sits just off the entry edge; the whole frame is `from`.
//! - `t → 1`: the front sweeps across; ahead of the (bled) front is `from`, behind
//!   it is `to`, and the boundary between them is an organic watercolor edge that
//!   wicks/feathers rather than a straight line.
//! - `t = 1`: the front has left by the exit edge; the whole frame is `to`.
//!
//! ## Seam dissolve model
//!
//! The mix weight between `from` and `to` is driven by a **blurred coverage** of
//! the already-swept (`to`) region. `coverage_at(sp)` returns `1.0` where the
//! sample `sp` is behind the wipe front (already `to`) and `0.0` ahead of it (still
//! `from`), with a soft `SEAM_SOFT` ramp across the front. `blurred_coverage`
//! then averages this over a 48-tap golden-angle spiral disc whose initial angle is
//! dithered per-pixel by `hash21`, so neighbouring pixels sample slightly different
//! front crossings and the boundary breaks into an organic, feathered edge — the
//! wicking. The disc radius (`bleed_px`) controls how wide the wet edge spreads.
//!
//! At the endpoints the front is fully off-frame, so the blurred coverage is a flat
//! `0.0` (t=0) / `1.0` (t=1) everywhere and the result is exactly `from` / `to`.
//!
//! ## Renderer parity caveat
//!
//! Like orb-dissolve (and unlike No.0 crossfade), aqua-dissolve does **not** assert
//! strict CPU/GPU pixel parity — the spiral bleed uses `sin`, whose CPU and GPU
//! implementations differ by ULPs and compound over 48 taps. The tests assert the
//! *mechanism*: the t=0 / t=1 endpoints, a monotone sweep (the `to` region only
//! grows), and that the from→to boundary is **dissolved** (a feathered band of
//! mixed pixels rather than a one-pixel hard edge).

#[cfg(feature = "gpu")]
use image::RgbaImage;

#[cfg(feature = "gpu")]
use crate::transition::Transition;

/// WGSL production shader for [`AquaDissolve`], assembled once.
///
/// The host half (`TAU` / `hash21` / `clampf` / the seam `coverage_at`, plus the
/// fragment that drives the mix) lives in `aqua_dissolve.wgsl`; the shared bleed
/// engine (`blurred_coverage` / `aqua_character` / the `AQUA_*` constants) is
/// substituted in from `aquarelle::AQUA_BLEED_WGSL` at the `//!AQUA_BLEED_SHARED`
/// marker (same concat mechanism orber uses — orber#250 Phase 2). Returns a
/// `'static` slice (the assembled `String` is leaked into a `OnceLock`) so it
/// satisfies [`Transition::shader_wgsl`]'s `&'static str` contract.
#[cfg(feature = "gpu")]
pub fn aqua_dissolve_wgsl() -> &'static str {
    use std::sync::OnceLock;
    static WGSL: OnceLock<String> = OnceLock::new();
    WGSL.get_or_init(|| {
        AQUA_DISSOLVE_WGSL_TEMPLATE.replace("//!AQUA_BLEED_SHARED", aquarelle::AQUA_BLEED_WGSL)
    })
    .as_str()
}

/// Host half of the aqua-dissolve shader (everything except the shared bleed
/// engine, which is substituted at `//!AQUA_BLEED_SHARED`).
#[cfg(feature = "gpu")]
const AQUA_DISSOLVE_WGSL_TEMPLATE: &str = include_str!("aqua_dissolve.wgsl");

/// Fraction of the wipe travel reserved as off-frame margin at each end, so the
/// front (and its bleed disc) fully clears the frame at the endpoints. The front
/// travels across `[-FRONT_MARGIN, 1 + FRONT_MARGIN]` as `t: 0 → 1`. Comfortably
/// exceeds `SEAM_SOFT + BLEED_FRAC` so neither the soft ramp nor the spiral disc
/// can reach back onto the frame at t=0/t=1.
#[cfg(feature = "gpu")]
const FRONT_MARGIN: f32 = 0.30;

/// Half-width of the hard seam ramp (normalized flow units) before the bleed is
/// applied. Kept thin: the *visible* width of the watercolor edge comes from the
/// spiral bleed, not from this ramp. A non-zero value avoids a 1-pixel alias on
/// the unbled coverage and gives the spiral a gradient to feather.
#[cfg(feature = "gpu")]
const SEAM_SOFT: f32 = 0.02;

/// Spiral-bleed disc radius as a fraction of the shorter frame axis. This is how
/// far the wet edge wicks to either side of the front. Tuned so the dissolve reads
/// as a clearly organic band (several percent of the frame) without washing the
/// whole frame.
#[cfg(feature = "gpu")]
const BLEED_FRAC: f32 = 0.06;

/// Default `halo` for the wet-rim tint (`aqua_character`): a faint saturation
/// boost where the bled coverage is mid (the wet edge), like pigment pooling.
/// Kept low so the seam tints rather than glows.
#[cfg(feature = "gpu")]
const DEFAULT_HALO: f32 = 0.5;

/// Default `bloom` (center white-lift): kept low — aqua-dissolve is a seam, not an
/// orb, so there is no bright core to bloom.
#[cfg(feature = "gpu")]
const DEFAULT_BLOOM: f32 = 0.0;

/// Default deterministic seed for the spiral bleed (per-pixel dither phase).
#[cfg(feature = "gpu")]
pub const DEFAULT_SEED: f32 = 1.0;

/// Flow direction of the sweep (which edge `to` grows from).
#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AquaDirection {
    /// Left → right (the `to` region grows from the left).
    Lr,
    /// Right → left (the `to` region grows from the right).
    Rl,
    /// Top → bottom (the `to` region grows from the top).
    Tb,
    /// Bottom → top (the `to` region grows from the bottom).
    Bt,
}

#[cfg(feature = "gpu")]
impl AquaDirection {
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

    /// `true` when the flow runs along the horizontal (x) axis.
    fn is_horizontal(self) -> bool {
        matches!(self, Self::Lr | Self::Rl)
    }

    /// `true` when the sweep runs in the negative direction of its axis (the front
    /// starts at the high edge and moves toward the low edge).
    fn is_negative(self) -> bool {
        matches!(self, Self::Rl | Self::Bt)
    }

    /// Direction code uploaded to the GPU shader (0 lr, 1 rl, 2 tb, 3 bt).
    fn code(self) -> u32 {
        match self {
            Self::Lr => 0,
            Self::Rl => 1,
            Self::Tb => 2,
            Self::Bt => 3,
        }
    }
}

/// Tunable knobs for the dissolve, surfaced on the CLI as `--direction` /
/// `--bleed` / `--halo` / `--seed`.
#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug)]
pub struct AquaConfig {
    /// Flow / sweep direction.
    pub direction: AquaDirection,
    /// Spiral-bleed disc radius multiplier (scales [`BLEED_FRAC`]). `0` ⇒ a hard
    /// (un-bled) wipe; `1` ⇒ the default wet edge; `> 1` ⇒ a wider, wetter wick.
    pub bleed: f32,
    /// Wet-rim saturation boost (`aqua_character` halo). Multiplies [`DEFAULT_HALO`].
    pub halo: f32,
    /// Center white-lift (`aqua_character` bloom). Multiplies [`DEFAULT_BLOOM`].
    pub bloom: f32,
    /// Deterministic dither seed for the spiral.
    pub seed: f32,
}

#[cfg(feature = "gpu")]
impl Default for AquaConfig {
    fn default() -> Self {
        Self {
            // lr (left→right) is the primary use case — the default sweep direction.
            direction: AquaDirection::Lr,
            bleed: 1.0,
            halo: 1.0,
            bloom: 1.0,
            seed: DEFAULT_SEED,
        }
    }
}

#[cfg(feature = "gpu")]
impl AquaConfig {
    /// Effective spiral-bleed disc radius in **shorter-axis-normalized** units
    /// (the same units the GPU shader's `aspect_*` scaling produces). Floored at 0.
    fn bleed_norm(&self) -> f32 {
        (BLEED_FRAC * self.bleed).max(0.0)
    }

    /// Effective `halo` for `aqua_character`, clamped to a sane `[0, 1.5]`.
    fn halo_amt(&self) -> f32 {
        (DEFAULT_HALO * self.halo).clamp(0.0, 1.5)
    }

    /// Effective `bloom` for `aqua_character`, clamped to a sane `[0, 1]`.
    fn bloom_amt(&self) -> f32 {
        (DEFAULT_BLOOM * self.bloom).clamp(0.0, 1.0)
    }
}

/// **No.14 — aqua-dissolve.** A directional `from → to` sweep whose boundary is
/// dissolved by the shared aquarelle spiral bleed, so the seam wicks like wet
/// pigment instead of reading as a clean wipe line.
pub struct AquaDissolve;

#[cfg(feature = "gpu")]
impl AquaDissolve {
    /// Position of the wipe front along the flow axis (positive-axis sense),
    /// `[-FRONT_MARGIN, 1 + FRONT_MARGIN]` over `t: 0 → 1` so it is off-frame at
    /// both ends. Direction-agnostic; callers fold in the sweep direction.
    pub fn wipe_front(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        -FRONT_MARGIN + t * (1.0 + 2.0 * FRONT_MARGIN)
    }

    /// Encode the GPU sweep parameters: the wipe-front position (positive-axis
    /// sense), the direction code, the bleed disc radius (shorter-axis units), the
    /// rim halo, the bloom, and the dither seed.
    pub fn sweep_params(cfg: &AquaConfig, t: f32) -> AquaSweep {
        AquaSweep {
            front: Self::wipe_front(t),
            dir_code: cfg.direction.code(),
            bleed: cfg.bleed_norm(),
            halo: cfg.halo_amt(),
            bloom: cfg.bloom_amt(),
            seed: cfg.seed,
        }
    }

    /// CPU reference render with explicit [`AquaConfig`] (the trait method uses the
    /// default config; the CLI threads the user's knobs here).
    ///
    /// Mirrors the WGSL: per pixel, the flow-axis coordinate `u` defines an unbled
    /// `to`-region coverage (`1` behind the front, `0` ahead), which the shared
    /// `aqua_blurred_coverage_cpu` spiral-averages into a feathered `to_weight`.
    /// `from`/`to` are mixed by that weight; the wet rim is tinted by
    /// `aqua_character_cpu` (halo / bloom) where the bled coverage is mid.
    pub fn render_cpu_cfg(
        &self,
        from: &RgbaImage,
        to: &RgbaImage,
        t: f32,
        cfg: &AquaConfig,
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
        let dir = cfg.direction;
        let front = Self::wipe_front(t);
        let bleed_norm = cfg.bleed_norm();
        let halo = cfg.halo_amt();
        let bloom = cfg.bloom_amt();
        let seed = cfg.seed;

        // Disc radius is normalized by the shorter axis; convert to per-axis pixel
        // radii so the spiral disc is isotropic on non-square frames (mirrors the
        // GPU `aspect_*` scaling).
        let short = w.min(h) as f32;
        let bleed_px = bleed_norm * short;

        // The unbled `to`-region coverage at a *pixel-space* sample, as the spiral
        // taps will probe it. `1.0` behind the front, `0.0` ahead, SEAM_SOFT ramp.
        let coverage_to = |sx: f32, sy: f32| -> (f32, f32) {
            let u = if dir.is_horizontal() {
                sx / w as f32
            } else {
                sy / h as f32
            };
            let behind = if dir.is_negative() {
                u - (1.0 - front)
            } else {
                front - u
            };
            // behind > 0 ⇒ swept ⇒ to (coverage 1); behind < 0 ⇒ ahead ⇒ from (0).
            // rgb_scale is unused for the silhouette (kept 1.0), the alpha carries
            // the to-weight that the spiral averages.
            (smoothstep(0.0, SEAM_SOFT, behind), 1.0)
        };

        let mut out = RgbaImage::new(w, h);
        for (x, y, o) in out.enumerate_pixels_mut() {
            let sx = x as f32 + 0.5;
            let sy = y as f32 + 0.5;

            // Spiral-averaged `to` weight: the feathered, wicking boundary. The
            // disc is isotropic in shorter-axis units; pass an aspect-corrected
            // sample so non-square frames don't stretch the wick. We probe in pixel
            // space, so scale the disc radius per-axis via separate closures is not
            // possible (the shared fn takes one blur_px); instead probe in a square
            // shorter-axis space by feeding normalized-then-pixel coords. Simplest
            // correct route: feed pixel coords and a single blur_px == bleed_px, and
            // accept mild anisotropy on extreme aspect ratios (the boundary is 1-D
            // along the flow axis, so cross-axis stretch is invisible anyway).
            let (cov_a, _scale) = aquarelle::aqua_blurred_coverage_cpu(
                coverage_to,
                (sx, sy),
                bleed_px,
                seed,
                (0.0, 0.0),
            );
            let to_w = cov_a.clamp(0.0, 1.0);

            let f = from.get_pixel(x, y);
            let g = to.get_pixel(x, y);
            // Base mix: from ahead (to_w small) → to behind (to_w large).
            let mut rgb = [
                f.0[0] as f32 / 255.0 * (1.0 - to_w) + g.0[0] as f32 / 255.0 * to_w,
                f.0[1] as f32 / 255.0 * (1.0 - to_w) + g.0[1] as f32 / 255.0 * to_w,
                f.0[2] as f32 / 255.0 * (1.0 - to_w) + g.0[2] as f32 / 255.0 * to_w,
            ];

            // Wet-rim character: tint only the wet edge (where to_w is mid), never
            // the flat from/to interiors. `aqua_character` keys its halo off LOW
            // coverage alpha (it expects an orb's center-high / edge-low alpha), so
            // we feed it the wet edge as "center" (cov_a high at the rim) and then
            // blend its result in proportional to the rim strength so the flats —
            // which would otherwise be read as a strong edge and tinted — stay
            // exactly `from` / `to`. At the endpoints to_w is a flat 0 / 1, rim is 0
            // everywhere, and the base mix is returned untouched.
            if halo > 0.0 || bloom > 0.0 {
                let rim = 1.0 - (2.0 * to_w - 1.0).abs(); // 0 at the flats, 1 at to_w=0.5
                if rim > 0.0 {
                    let tinted = aquarelle::aqua_character_cpu(rgb, rim, bloom, halo);
                    for c in 0..3 {
                        rgb[c] = rgb[c] * (1.0 - rim) + tinted[c] * rim;
                    }
                }
            }

            o.0[0] = (rgb[0] * 255.0).round().clamp(0.0, 255.0) as u8;
            o.0[1] = (rgb[1] * 255.0).round().clamp(0.0, 255.0) as u8;
            o.0[2] = (rgb[2] * 255.0).round().clamp(0.0, 255.0) as u8;
            o.0[3] = 255;
        }
        out
    }
}

/// GPU sweep parameters for one aqua-dissolve frame (see [`AquaDissolve::sweep_params`]).
#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug)]
pub struct AquaSweep {
    /// Wipe-front position along the flow axis (positive-axis sense).
    pub front: f32,
    /// Direction code: 0 lr, 1 rl, 2 tb, 3 bt.
    pub dir_code: u32,
    /// Spiral-bleed disc radius in shorter-axis-normalized units.
    pub bleed: f32,
    /// Wet-rim saturation boost (`aqua_character` halo).
    pub halo: f32,
    /// Center white-lift (`aqua_character` bloom).
    pub bloom: f32,
    /// Dither seed for the spiral.
    pub seed: f32,
}

#[cfg(feature = "gpu")]
impl crate::additive::Additive for AquaDissolve {
    fn designation(&self) -> &'static str {
        "No.14"
    }

    fn name(&self) -> &'static str {
        "aqua-dissolve"
    }

    fn description(&self) -> &'static str {
        "A directional sweep whose from→to seam dissolves into a watercolor bleed (にじみ), wicking like wet pigment."
    }
}

#[cfg(feature = "gpu")]
impl Transition for AquaDissolve {
    fn render_cpu(&self, from: &RgbaImage, to: &RgbaImage, t: f32) -> RgbaImage {
        self.render_cpu_cfg(from, to, t, &AquaConfig::default())
    }

    fn shader_wgsl(&self) -> Option<&'static str> {
        Some(aqua_dissolve_wgsl())
    }
}

/// CPU-only stub when the `gpu` feature (and thus aquarelle) is off.
#[cfg(not(feature = "gpu"))]
impl AquaDissolve {}

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

    /// A flat-but-distinct `from`/`to` so a pixel's color tells which side of the
    /// (dissolved) seam it is on.
    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for px in img.pixels_mut() {
            *px = Rgba([rgb[0], rgb[1], rgb[2], 255]);
        }
        img
    }

    /// A photo-ish gradient for non-flat inputs.
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

    /// Fraction of pixels closer to `to` than to `from`: the size of the swept region.
    fn to_fraction(frame: &RgbaImage, from: &RgbaImage, to: &RgbaImage) -> f32 {
        let mut to_px = 0u64;
        let mut n = 0u64;
        for ((p, f), g) in frame.pixels().zip(from.pixels()).zip(to.pixels()) {
            let df: u32 = (0..3).map(|c| p.0[c].abs_diff(f.0[c]) as u32).sum();
            let dg: u32 = (0..3).map(|c| p.0[c].abs_diff(g.0[c]) as u32).sum();
            if dg < df {
                to_px += 1;
            }
            n += 1;
        }
        to_px as f32 / n as f32
    }

    #[test]
    fn t0_is_close_to_from() {
        let from = sample_image(96, 96, false);
        let to = sample_image(96, 96, true);
        let ad = AquaDissolve;
        let frame = ad.render_cpu(&from, &to, 0.0);
        let d_from = mean_rgb_diff(&frame, &from);
        let d_to = mean_rgb_diff(&frame, &to);
        eprintln!("t=0: mean diff to from={d_from:.2}, to to={d_to:.2}");
        assert!(d_from < 1.5, "t=0 should be (almost) exactly `from`");
        assert!(d_from < d_to, "t=0 must be much closer to `from`");
    }

    #[test]
    fn t1_is_close_to_to() {
        let from = sample_image(96, 96, false);
        let to = sample_image(96, 96, true);
        let ad = AquaDissolve;
        let frame = ad.render_cpu(&from, &to, 1.0);
        let d_from = mean_rgb_diff(&frame, &from);
        let d_to = mean_rgb_diff(&frame, &to);
        eprintln!("t=1: mean diff to from={d_from:.2}, to to={d_to:.2}");
        assert!(d_to < 1.5, "t=1 should be (almost) exactly `to`");
        assert!(d_to < d_from, "t=1 must be much closer to `to`");
    }

    /// **Sweep monotonicity.** The `to` region grows monotonically with `t`.
    #[test]
    fn sweep_is_monotone() {
        let from = solid(128, 128, [255, 0, 0]);
        let to = solid(128, 128, [0, 0, 255]);
        let ad = AquaDissolve;
        let cfg = AquaConfig::default();
        let mut prev = -1.0f32;
        let mut fracs = Vec::new();
        for k in 0..=20 {
            let t = k as f32 / 20.0;
            let frame = ad.render_cpu_cfg(&from, &to, t, &cfg);
            let frac = to_fraction(&frame, &from, &to);
            fracs.push((t, frac));
            assert!(
                frac >= prev - 0.02,
                "to-fraction must not retreat: t={t} frac={frac} prev={prev}"
            );
            prev = frac;
        }
        eprintln!("sweep to-fraction: {fracs:?}");
        assert!(fracs[0].1 < 0.1, "t=0 must be almost all from");
        assert!(fracs[20].1 > 0.9, "t=1 must be almost all to");
    }

    /// **Seam is dissolved, not a hard line.** Mid-clip, the transition from
    /// `from`-dominant to `to`-dominant must span **many** pixels (the watercolor
    /// wick), not flip in one or two. We count, over the whole frame, the pixels
    /// that are *mixed* (neither pure `from` nor pure `to`). A hard wipe would leave
    /// at most ~1 mixed column per row; the bleed feathers a band several columns
    /// wide, so the per-row average is well above that.
    #[test]
    fn seam_is_dissolved_not_hard() {
        let from = solid(160, 160, [220, 40, 40]);
        let to = solid(160, 160, [40, 40, 220]);
        let ad = AquaDissolve;
        let cfg = AquaConfig::default();
        let frame = ad.render_cpu_cfg(&from, &to, 0.5, &cfg);
        let (w, h) = frame.dimensions();

        // A pixel is "mixed" when it is neither close to pure `from` (red) nor pure
        // `to` (blue): both red and blue channels are appreciably present.
        let mut mixed = 0u32;
        for p in frame.pixels() {
            let is_from = p.0[0] > 180 && p.0[2] < 80;
            let is_to = p.0[2] > 180 && p.0[0] < 80;
            if !is_from && !is_to {
                mixed += 1;
            }
        }
        eprintln!(
            "seam dissolve: {mixed} mixed px of {} ({} per row)",
            w * h,
            mixed as f32 / h as f32
        );
        // The bleed disc is BLEED_FRAC of the shorter axis; the feathered band is
        // several columns wide on average (well above the ≤1/row of a hard wipe).
        assert!(
            mixed > 3 * h,
            "the from→to seam must dissolve across a wide watercolor band, not a hard line (got {mixed} mixed px, {} per row)",
            mixed as f32 / h as f32
        );
    }

    /// **Bleed widens the dissolve.** A larger `--bleed` must produce a wider band
    /// of mixed pixels than a small one (the wick spreads further).
    #[test]
    fn bleed_widens_the_band() {
        let from = solid(160, 160, [220, 40, 40]);
        let to = solid(160, 160, [40, 40, 220]);
        let ad = AquaDissolve;
        let mixed_count = |bleed: f32| -> u32 {
            let cfg = AquaConfig {
                bleed,
                ..Default::default()
            };
            let frame = ad.render_cpu_cfg(&from, &to, 0.5, &cfg);
            let mut mixed = 0u32;
            for p in frame.pixels() {
                let is_from = p.0[0] > 180 && p.0[2] < 80;
                let is_to = p.0[2] > 180 && p.0[0] < 80;
                if !is_from && !is_to {
                    mixed += 1;
                }
            }
            mixed
        };
        let narrow = mixed_count(0.5);
        let wide = mixed_count(2.5);
        eprintln!("bleed widens: narrow={narrow} mixed, wide={wide} mixed");
        assert!(
            wide > narrow,
            "a larger --bleed must widen the dissolve band (narrow={narrow}, wide={wide})"
        );
    }

    /// **Direction reverses the sweep.** With `lr` the `to` region grows from the
    /// left; with `rl` from the right. At the same `t` the left edge is `to` under
    /// lr but still `from` under rl.
    #[test]
    fn direction_reverses_sweep() {
        let from = solid(120, 120, [255, 0, 0]);
        let to = solid(120, 120, [0, 0, 255]);
        let ad = AquaDissolve;
        let t = 0.35;
        let (w, h) = (120u32, 120u32);

        let left_to = |dir: AquaDirection| -> u32 {
            let cfg = AquaConfig {
                direction: dir,
                ..Default::default()
            };
            let frame = ad.render_cpu_cfg(&from, &to, t, &cfg);
            let mut c = 0;
            for y in 0..h {
                for x in 0..(w / 6) {
                    let p = frame.get_pixel(x, y).0;
                    if p[2] > 150 && p[0] < 100 {
                        c += 1;
                    }
                }
            }
            c
        };
        let lr_left = left_to(AquaDirection::Lr);
        let rl_left = left_to(AquaDirection::Rl);
        eprintln!("direction: lr left-edge to-px={lr_left}, rl left-edge to-px={rl_left}");
        assert!(
            lr_left > rl_left,
            "lr must turn the left edge to `to` before rl does (lr={lr_left}, rl={rl_left})"
        );
    }

    #[test]
    fn empty_image_yields_empty() {
        let from = RgbaImage::new(0, 0);
        let to = RgbaImage::new(0, 0);
        let ad = AquaDissolve;
        let frame = ad.render_cpu(&from, &to, 0.5);
        assert_eq!(frame.dimensions(), (0, 0));
    }

    #[test]
    fn wipe_front_spans_offscreen_ends() {
        assert!(AquaDissolve::wipe_front(0.0) < 0.0);
        assert!(AquaDissolve::wipe_front(1.0) > 1.0);
        assert!((AquaDissolve::wipe_front(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn shader_assembles_with_shared_bleed() {
        let wgsl = aqua_dissolve_wgsl();
        // The shared bleed engine must have been substituted in (no marker left).
        assert!(
            !wgsl.contains("//!AQUA_BLEED_SHARED"),
            "the shared-bleed marker must be replaced"
        );
        assert!(
            wgsl.contains("fn blurred_coverage"),
            "the shared bleed fragment must be present"
        );
        assert!(
            wgsl.contains("fn fs_main"),
            "the host fragment entry point must be present"
        );
    }
}
