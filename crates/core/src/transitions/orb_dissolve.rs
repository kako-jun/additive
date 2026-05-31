//! No.13 — orb-dissolve (flagship), **conveyor sweep-wipe**.
//!
//! A band of color orbs — flowing on orber's one-way conveyor — sweeps across the
//! frame. The band is a strip *perpendicular* to the flow direction; it advances
//! from the entry edge to the exit edge as `t: 0 → 1`. Ahead of the band the base
//! is still `from`; behind it the base is already `to`. The `from → to` boundary
//! (the *seam*) always sits **inside** the orb band, so it is never directly
//! visible — the orbs (carrying `from`'s palette) wash over it like a wave and
//! leave `to` in their wake.
//!
//! ## Timeline
//!
//! - `t = 0`: the band sits just off the entry edge; the whole frame is `from`
//!   (orbs ≈ absent on-screen).
//! - `t → 1`: the band sweeps across; the region it has passed shows `to`, the
//!   region ahead still shows `from`, and the seam between them rides inside the
//!   band.
//! - `t = 1`: the band has left by the exit edge; the whole frame is `to`.
//!
//! This is a *wipe* — directional, single-pass — not a translucent cross-fade and
//! not a global occlusion pulse. The orbs never cover the whole frame at once;
//! they cover one perpendicular slice (the band) at a time.
//!
//! ## Coverage model
//!
//! Orbs are placed on a **jittered grid** whose *perpendicular* rows tile the
//! frame's cross-axis (so the band is gap-free across its full width) and whose
//! *flow-axis* columns are packed into the band's thickness (so the band is opaque
//! through its depth). The band center tracks the **wipe front** `p(t)`; orbs also
//! ride a one-way conveyor drift along the flow axis (so they read as *flowing*,
//! not as a rigid wall). Outside the band the orb opacity falls to ~0.
//!
//! ## Renderer parity caveat
//!
//! Unlike No.0 crossfade, orb-dissolve does **not** assert strict pixel parity
//! between the CPU (tiny-skia) and GPU (WGSL) paths — orber itself split over
//! exactly that two-rasterizer mismatch. The tests assert the *mechanism*: the
//! t=0 / t=1 endpoints, monotone progress of the `to` region (the front sweeps one
//! way), and that the from/to seam is covered by orbs (no raw hard seam shows).

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
/// `orb_dissolve.wgsl` and `gpu::MAX_ORBS`).
#[cfg(feature = "gpu")]
pub const MAX_ORBS: usize = 128;

/// Default orb count: dense enough that the sweeping band is gap-free across the
/// cross-axis at the default `--orb-size`.
#[cfg(feature = "gpu")]
pub const DEFAULT_COUNT: u32 = 96;

/// Default conveyor speed multiplier (one-way drift of the orbs along the flow).
#[cfg(feature = "gpu")]
pub const DEFAULT_SPEED: f32 = 1.0;

/// Default orb-size multiplier (scales disc radius and, with it, the band width).
#[cfg(feature = "gpu")]
pub const DEFAULT_ORB_SIZE: f32 = 1.0;

/// How much neighbouring orbs overlap, as a multiple of the grid cell's
/// half-diagonal. `> 1` guarantees the soft discs' opaque cores meet with no gap.
#[cfg(feature = "gpu")]
const COVER_OVERLAP: f32 = 1.85;

/// The fraction of the wipe travel taken up by the band's half-thickness margins
/// at each end, so that at `t = 0` the band is fully off the entry edge and at
/// `t = 1` it is fully off the exit edge. The front therefore travels across
/// `[-MARGIN, 1 + MARGIN]` as `t: 0 → 1`. Chosen to comfortably exceed
/// `BAND_HALF_MAX + radius` so the band fully clears the frame at both endpoints
/// (otherwise a half-faded disc would smear the entry/exit edge).
#[cfg(feature = "gpu")]
const FRONT_MARGIN: f32 = 0.34;

/// Upper bound on the band's half-thickness (normalized flow units), so the
/// [`FRONT_MARGIN`] can always carry the whole band off-frame at the endpoints.
#[cfg(feature = "gpu")]
const BAND_HALF_MAX: f32 = 0.20;

/// Flow direction of the conveyor / the axis the wipe sweeps along.
#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbDirection {
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

    /// `true` when the flow runs along the horizontal (x) axis.
    fn is_horizontal(self) -> bool {
        matches!(self, Self::Lr | Self::Rl)
    }

    /// `true` when the sweep runs in the negative direction of its axis (the
    /// front starts at the high edge and moves toward the low edge).
    fn is_negative(self) -> bool {
        matches!(self, Self::Rl | Self::Bt)
    }
}

/// Tunable knobs for the sweep band, surfaced on the CLI as `--count` /
/// `--speed` / `--direction` / `--orb-size`.
#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug)]
pub struct OrbConfig {
    /// Number of orbs (clamped to `[1, MAX_ORBS]`). Drives the band's coverage grid.
    pub count: u32,
    /// Conveyor speed multiplier (how fast the orbs drift along the flow).
    pub speed: f32,
    /// Flow / sweep direction.
    pub direction: OrbDirection,
    /// Orb-size multiplier; scales disc radius and the band thickness.
    pub orb_size: f32,
}

#[cfg(feature = "gpu")]
impl Default for OrbConfig {
    fn default() -> Self {
        Self {
            count: DEFAULT_COUNT,
            speed: DEFAULT_SPEED,
            // rl (right→left) is the primary use case — the default sweep direction.
            direction: OrbDirection::Rl,
            orb_size: DEFAULT_ORB_SIZE,
        }
    }
}

/// Per-orb data uploaded to the GPU (and used to drive the CPU path too).
///
/// `pos` is the orb center in normalized `[0,1]^2` UV, `radius` is normalized by
/// the shorter axis, `color` is straight sRGB `[0,1]`, `alpha` the orb's current
/// band opacity.
#[cfg(feature = "gpu")]
#[derive(Clone, Copy)]
pub struct OrbInstance {
    pub pos: [f32; 2],
    pub radius: f32,
    pub alpha: f32,
    pub color: [f32; 3],
}

/// **No.13 — orb-dissolve.** A band of color orbs flows across the frame on a
/// one-way conveyor, sweeping the base from `from` (ahead of the band) to `to`
/// (behind it) with the seam always hidden inside the band.
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

    /// Position of the wipe front along the flow axis (in normalized flow-axis
    /// coordinates, increasing in the *positive* axis direction), at time `t`.
    ///
    /// Travels linearly from `-FRONT_MARGIN` to `1 + FRONT_MARGIN` so the band is
    /// off-frame at both ends. This is **direction-agnostic**: the value is in the
    /// positive-axis sense; callers fold in the sweep direction.
    pub fn wipe_front(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        -FRONT_MARGIN + t * (1.0 + 2.0 * FRONT_MARGIN)
    }

    /// The base-layer `from`-weight at flow-axis coordinate `u ∈ [0,1]` and time
    /// `t`, for the given direction. `1.0` ⇒ pure `from` (ahead of the front),
    /// `0.0` ⇒ pure `to` (behind it). The transition across the front is a hard
    /// step (the orb band hides it); a tiny `SEAM_SOFT` ramp avoids a 1-pixel
    /// aliasing line but stays far thinner than the band.
    ///
    /// For a positive-axis sweep (`lr`/`tb`) "behind" means `u < front` (the low
    /// edge fills with `to` first). For a negative-axis sweep (`rl`/`bt`) the
    /// coordinate is mirrored so the high edge fills first.
    pub fn base_from_weight_at(u: f32, t: f32, dir: OrbDirection) -> f32 {
        const SEAM_SOFT: f32 = 0.012;
        let front = Self::wipe_front(t);
        // Fold direction: measure how far `u` is *behind* the front along the
        // sweep. For a positive sweep, behind = front - u. For negative, mirror.
        let behind = if dir.is_negative() {
            u - (1.0 - front)
        } else {
            front - u
        };
        // behind > 0 ⇒ already swept ⇒ to (from-weight 0).
        // behind < 0 ⇒ ahead ⇒ from (from-weight 1).
        smoothstep(0.0, SEAM_SOFT, -behind)
    }

    /// Compute the live orb instances at time `t` for the given `cfg`, sampling
    /// each orb's color from `from`.
    ///
    /// Orbs tile a jittered grid: its *cross-axis* rows span the full frame width
    /// (gap-free coverage of the band) and its *flow-axis* columns are packed into
    /// the band's thickness, centered on the wipe front. Each orb's opacity is a
    /// band envelope (opaque inside the band, ~0 outside), and the whole field
    /// rides a one-way conveyor drift along the flow axis at `cfg.speed`.
    pub fn orb_instances(from: &RgbaImage, cfg: &OrbConfig, t: f32) -> Vec<OrbInstance> {
        let t = t.clamp(0.0, 1.0);
        let count = cfg.count.clamp(1, MAX_ORBS as u32) as usize;
        let size = cfg.orb_size.max(0.0);

        // Grid: many cross-axis cells (cover the band width), a few flow-axis
        // cells (give the band depth). Aim for a wide, thin lattice.
        let cross = ((count as f32 * 4.0).sqrt().ceil() as usize).clamp(1, count);
        let depth = count.div_ceil(cross).max(1);

        // Band half-thickness along the flow axis (normalized flow units). Scales
        // with orb-size and (inversely) with how many cross cells we have so a
        // single column of discs still spans it. Clamped so it stays a *band*.
        let band_half = (0.10 * size * (depth as f32).max(1.0).sqrt()).clamp(0.04, BAND_HALF_MAX);

        // Cross-axis cell geometry (the band's full width is the [0,1] cross-span).
        let cross_cell = 1.0 / cross as f32;
        let depth_cell = if depth > 1 {
            (2.0 * band_half) / depth as f32
        } else {
            2.0 * band_half
        };
        // Coverage radius from the larger of the two cell half-extents so discs
        // overlap along both lattice directions.
        let cross_hw = 0.5 * cross_cell;
        let depth_hw = 0.5 * depth_cell;
        let half_diag = (cross_hw * cross_hw + depth_hw * depth_hw).sqrt();
        let radius = half_diag * COVER_OVERLAP * size;

        let front = Self::wipe_front(t);
        let dir = cfg.direction;
        // One-way conveyor drift along the flow axis (wrapped within the band so
        // the band stays put while the discs visibly travel through it).
        let drift = fract(cfg.speed * t);

        let (iw, ih) = from.dimensions();
        let pool = Self::orb_pool(from);

        let mut out = Vec::with_capacity(count);
        for idx in 0..count {
            let cx = idx % cross; // cross-axis cell
            let cz = idx / cross; // flow-axis (depth) cell

            // Cross-axis home in [0,1].
            let cross_home = (cx as f32 + 0.5) / cross as f32;
            // Flow-axis position within the band: distribute depth cells across
            // [-band_half, +band_half] around the front, then add the drift.
            let depth_norm = if depth > 1 {
                (cz as f32 + 0.5) / depth as f32 - 0.5 // [-0.5, 0.5)
            } else {
                0.0
            };
            let along_band = depth_norm * 2.0 * band_half + (drift - 0.5) * depth_cell;

            // Small deterministic jitter (golden-ratio) so the lattice doesn't read
            // as rigid; a fraction of a cell each way.
            let jx = (fract(0.137 + idx as f32 * 0.618_034) - 0.5) * cross_cell * 0.7;
            let jz = (fract(0.731 + idx as f32 * 0.381_966) - 0.5) * depth_cell * 0.7;
            let cross_pos = (cross_home + jx).clamp(0.0, 1.0);

            // Flow-axis position: front + along_band + jitter, in positive-axis
            // sense, then fold the direction (negative axis mirrors the front).
            let flow_signed = front + along_band + jz;
            let flow_pos = if dir.is_negative() {
                1.0 - flow_signed
            } else {
                flow_signed
            };

            // Map (flow, cross) -> (x, y) by axis.
            let (px, py) = if dir.is_horizontal() {
                (flow_pos, cross_pos)
            } else {
                (cross_pos, flow_pos)
            };

            // Band opacity: opaque while the orb is inside the band, fading at the
            // band edges. Distance of this orb from the front along the flow axis.
            let dist_from_front = (flow_signed - front).abs();
            let band_alpha = 1.0 - smoothstep(band_half * 0.6, band_half * 1.15, dist_from_front);
            // On-frame fade: an orb whose flow-axis position has left the frame
            // (entry edge < 0 at t→0, exit edge > 1 at t→1) fades out, so the band
            // is gone at both endpoints and never lingers as an edge smear. Margin
            // ≈ the orb radius so a disc fully clears before it is cut.
            let edge = radius;
            let on_frame = smoothstep(-edge, edge, flow_signed)
                * (1.0 - smoothstep(1.0 - edge, 1.0 + edge, flow_signed));
            let alpha = (band_alpha * on_frame).clamp(0.0, 1.0);

            // Color: sample `from` at the orb's cross-home along the front (so the
            // band carries from's palette). Fall back to a cluster / mid-grey.
            let (su, sv) = if dir.is_horizontal() {
                (front.clamp(0.0, 1.0), cross_pos)
            } else {
                (cross_pos, front.clamp(0.0, 1.0))
            };
            let color = sample_color(from, iw, ih, su, sv, &pool, idx);

            out.push(OrbInstance {
                pos: [px.clamp(-0.5, 1.5), py.clamp(-0.5, 1.5)],
                radius,
                alpha,
                color,
            });
        }
        out
    }

    /// Build the GPU orb array for `from` at time `t`.
    pub fn gpu_orbs(from: &RgbaImage, cfg: &OrbConfig, t: f32) -> Vec<crate::gpu::GpuOrb> {
        Self::orb_instances(from, cfg, t)
            .into_iter()
            .map(|o| crate::gpu::GpuOrb {
                pos_radius_alpha: [o.pos[0], o.pos[1], o.radius, o.alpha],
                color: [o.color[0], o.color[1], o.color[2], 0.0],
            })
            .collect()
    }

    /// Encode the base-layer sweep parameters for the GPU shader: the wipe front
    /// position (positive-axis sense) and a direction code (0 lr, 1 rl, 2 tb, 3 bt).
    pub fn sweep_params(cfg: &OrbConfig, t: f32) -> (f32, u32) {
        let code = match cfg.direction {
            OrbDirection::Lr => 0u32,
            OrbDirection::Rl => 1,
            OrbDirection::Tb => 2,
            OrbDirection::Bt => 3,
        };
        (Self::wipe_front(t), code)
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
        "A band of orbs sweeps across, wiping the base from `from` (ahead) to `to` (behind), seam hidden in the band."
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
    /// Base layer is a directional sweep (`from` ahead of the front, `to` behind);
    /// the orb band is drawn as soft discs on top, matching the WGSL falloff.
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
        let dir = cfg.direction;

        // Base layer: directional from→to sweep. Flow-axis coordinate `u` per pixel.
        let mut out = RgbaImage::new(w, h);
        for (x, y, o) in out.enumerate_pixels_mut() {
            let ux = (x as f32 + 0.5) / w as f32;
            let uy = (y as f32 + 0.5) / h as f32;
            let u = if dir.is_horizontal() { ux } else { uy };
            let from_w = Self::base_from_weight_at(u, t, dir);
            let f = from.get_pixel(x, y);
            let g = to.get_pixel(x, y);
            for c in 0..3 {
                let v = f.0[c] as f32 * from_w + g.0[c] as f32 * (1.0 - from_w);
                o.0[c] = v.round().clamp(0.0, 255.0) as u8;
            }
            o.0[3] = 255;
        }

        // Orb band. Drawn with the same soft-disc falloff as the WGSL.
        let instances = Self::orb_instances(from, cfg, t);
        let short = w.min(h) as f32;
        let aspect_x = w as f32 / short;
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
                // Wrap only along the *cross* axis (the band tiles the cross-span);
                // the flow axis is open (the band enters and leaves the frame).
                let (dxr, dyr) = if dir.is_horizontal() {
                    ((ux - o.pos[0]), wrap_delta(uy - o.pos[1]))
                } else {
                    (wrap_delta(ux - o.pos[0]), (uy - o.pos[1]))
                };
                let dx = dxr * aspect_x;
                let dy = dyr * aspect_y;
                let d = (dx * dx + dy * dy).sqrt();
                let inner = o.radius * 0.7;
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

/// CPU-only stub when the `gpu` feature (and thus orber-core) is off.
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
        let x = ((u.clamp(0.0, 1.0) * iw as f32) as u32).min(iw - 1);
        let y = ((v.clamp(0.0, 1.0) * ih as f32) as u32).min(ih - 1);
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
/// unit torus. Used so an orb near one cross-edge of the band also covers the
/// opposite cross-edge (no seam at the band's cross wrap).
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

    /// A flat-but-distinct `from` (all red) and `to` (all blue) so a pixel's
    /// base color cleanly tells which side of the seam it is on.
    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for px in img.pixels_mut() {
            *px = Rgba([rgb[0], rgb[1], rgb[2], 255]);
        }
        img
    }

    /// A photo-ish gradient (varied colors) for palette / drift tests.
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

    /// Fraction of pixels closer to `to` (blue) than to `from` (red): the size of
    /// the already-swept region. Uses solid red/blue images.
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
    fn orb_pool_is_non_empty() {
        let from = sample_image(96, 96, false);
        let pool = OrbDissolve::orb_pool(&from);
        assert!(!pool.is_empty(), "clustering should yield a non-empty pool");
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
        assert!(d_from < 3.0, "t=0 should be (almost) exactly `from`");
        assert!(d_from < d_to, "t=0 must be much closer to `from`");
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
        assert!(d_to < 3.0, "t=1 should be (almost) exactly `to`");
        assert!(d_to < d_from, "t=1 must be much closer to `to`");
    }

    /// **Sweep monotonicity.** The `to` region grows monotonically with `t` — the
    /// front advances one way and never retreats.
    #[test]
    fn sweep_is_monotone() {
        let from = solid(128, 128, [255, 0, 0]);
        let to = solid(128, 128, [0, 0, 255]);
        let od = OrbDissolve;
        let cfg = OrbConfig::default();
        let mut prev = -1.0f32;
        let mut fracs = Vec::new();
        for k in 0..=20 {
            let t = k as f32 / 20.0;
            let frame = od.render_cpu_cfg(&from, &to, t, &cfg);
            let frac = to_fraction(&frame, &from, &to);
            fracs.push((t, frac));
            // Allow a tiny epsilon for orb-color noise near the seam.
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

    /// **Seam coverage.** At mid-clip the from↔to boundary must be hidden under
    /// the orb band. We use a solid GREEN `from` and solid BLUE `to`: the orbs
    /// carry `from`'s color (green), the base *behind* the front is blue. If the
    /// band hides the seam, the seam column shows green (orbs) over what would
    /// otherwise be a raw green→blue hard edge. We assert the seam column carries
    /// the orb (green) color rather than exposing the bare base.
    #[test]
    fn seam_is_covered_by_orbs() {
        let from = solid(160, 160, [0, 220, 0]);
        let to = solid(160, 160, [0, 0, 255]);
        let od = OrbDissolve;
        let cfg = OrbConfig::default();
        let t = 0.5;
        let frame = od.render_cpu_cfg(&from, &to, t, &cfg);

        // Seam flow-axis position (lr => x). front in positive-axis sense.
        let front = OrbDissolve::wipe_front(t);
        let (w, h) = frame.dimensions();
        let seam_x = ((front.clamp(0.0, 1.0) * w as f32) as u32).min(w - 1);

        // Along the seam column: count pixels that are green-dominant (orb color).
        // Behind the front the bare base would be blue; orbs covering the seam
        // paint it green. High green coverage ⇒ the seam is hidden by the band.
        let mut green = 0u32;
        for y in 0..h {
            let p = frame.get_pixel(seam_x, y).0;
            if p[1] > 120 && p[1] > p[2] {
                green += 1;
            }
        }
        let frac = green as f32 / h as f32;
        eprintln!("seam_x={seam_x} (front={front:.3}): orb (green) coverage = {frac:.3}");
        assert!(
            frac > 0.85,
            "the from/to seam must be hidden under the orb band (covered {frac:.2})"
        );
    }

    /// **Direction reverses the sweep.** With `lr` the `to` region grows from the
    /// left (low x); with `rl` it grows from the right (high x). At the same `t`,
    /// the left half is `to`-dominant under lr but `from`-dominant under rl.
    #[test]
    fn direction_reverses_sweep() {
        let from = solid(120, 120, [255, 0, 0]);
        let to = solid(120, 120, [0, 0, 255]);
        let od = OrbDissolve;
        let t = 0.35; // front past the left third
        let (w, h) = (120u32, 120u32);

        let lr = od.render_cpu_cfg(
            &from,
            &to,
            t,
            &OrbConfig {
                direction: OrbDirection::Lr,
                ..Default::default()
            },
        );
        let rl = od.render_cpu_cfg(
            &from,
            &to,
            t,
            &OrbConfig {
                direction: OrbDirection::Rl,
                ..Default::default()
            },
        );

        // Count `to` pixels in the left quarter for each direction (avoid the band
        // region by sampling near the edges).
        let left_to = |frame: &RgbaImage| -> u32 {
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
        let lr_left = left_to(&lr);
        let rl_left = left_to(&rl);
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
        let od = OrbDissolve;
        let frame = od.render_cpu(&from, &to, 0.5);
        assert_eq!(frame.dimensions(), (0, 0));
    }

    #[test]
    fn orbs_drift_along_flow() {
        let from = sample_image(96, 96, false);
        let cfg = OrbConfig::default();
        let a = OrbDissolve::orb_instances(&from, &cfg, 0.45);
        let b = OrbDissolve::orb_instances(&from, &cfg, 0.55);
        assert!(!a.is_empty() && !b.is_empty());
        let moved = a
            .iter()
            .zip(b.iter())
            .any(|(p, q)| (p.pos[0] - q.pos[0]).abs() > 1e-3 || (p.pos[1] - q.pos[1]).abs() > 1e-3);
        assert!(moved, "orbs must drift / advance over time");
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
        // lr sweeps along x: the front moves the orb band along x as t grows.
        let lr0 = OrbDissolve::orb_instances(
            &from,
            &OrbConfig {
                direction: OrbDirection::Lr,
                ..Default::default()
            },
            0.3,
        );
        let lr1 = OrbDissolve::orb_instances(
            &from,
            &OrbConfig {
                direction: OrbDirection::Lr,
                ..Default::default()
            },
            0.7,
        );
        let tb0 = OrbDissolve::orb_instances(
            &from,
            &OrbConfig {
                direction: OrbDirection::Tb,
                ..Default::default()
            },
            0.3,
        );
        let tb1 = OrbDissolve::orb_instances(
            &from,
            &OrbConfig {
                direction: OrbDirection::Tb,
                ..Default::default()
            },
            0.7,
        );
        // Average flow-axis position must advance more along x (lr) than y, and
        // vice-versa for tb.
        let avg_x = |v: &[OrbInstance]| v.iter().map(|o| o.pos[0]).sum::<f32>() / v.len() as f32;
        let avg_y = |v: &[OrbInstance]| v.iter().map(|o| o.pos[1]).sum::<f32>() / v.len() as f32;
        let lr_dx = (avg_x(&lr1) - avg_x(&lr0)).abs();
        let tb_dy = (avg_y(&tb1) - avg_y(&tb0)).abs();
        assert!(lr_dx > 0.1, "lr front must advance along x ({lr_dx})");
        assert!(tb_dy > 0.1, "tb front must advance along y ({tb_dy})");
    }

    #[test]
    fn wipe_front_spans_offscreen_ends() {
        // At t=0 the front is off the entry edge (<0); at t=1 off the exit (>1).
        assert!(OrbDissolve::wipe_front(0.0) < 0.0);
        assert!(OrbDissolve::wipe_front(1.0) > 1.0);
        assert!((OrbDissolve::wipe_front(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn base_weight_ahead_is_from_behind_is_to() {
        // lr at t=0.5: front≈0.5. Pixels left of front are `to` (weight 0), right
        // of front are `from` (weight 1).
        let dir = OrbDirection::Lr;
        assert!(OrbDissolve::base_from_weight_at(0.1, 0.5, dir) < 0.01); // behind ⇒ to
        assert!(OrbDissolve::base_from_weight_at(0.9, 0.5, dir) > 0.99); // ahead ⇒ from
                                                                         // rl mirrors: left of (1-front) is `from`, right is `to`.
        let dir = OrbDirection::Rl;
        assert!(OrbDissolve::base_from_weight_at(0.1, 0.5, dir) > 0.99);
        assert!(OrbDissolve::base_from_weight_at(0.9, 0.5, dir) < 0.01);
    }
}
