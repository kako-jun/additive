//! ADDITIVE-13 — pure rendering core for the "additive" series.
//!
//! ADDITIVE-13 is a *catalogue of image effects* ("additives"). Each additive
//! shares an [`Additive`] identity (an E-number designation + a stable name) and
//! is rendered as one of two kinds:
//!
//! - a [`Transition`] — a two-image time function `(from, to, t) -> RGBA frame`,
//!   `t` in `0.0..=1.0` (No.0 crossfade, No.13 orb-dissolve, …); or
//! - a [`Generator`] — source synthesis from zero or one image (the parts /
//!   material generators #19/#20/#21), which has no meaningful `to`.
//!
//! The catalogue ([`all`] / [`by_name`]) returns the [`AdditiveItem`] union so a
//! caller can list every entry uniformly yet drive the right render path. The
//! renderer in this crate is the **reference** CPU path; the production renderer
//! is wgpu (Rust + WGSL), shared by the native CLI and the browser so both
//! produce identical output (issue #1).

pub mod additive;
pub mod generator;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod transition;
pub mod transitions;

pub use additive::{all, by_name, Additive, AdditiveItem};
pub use generator::Generator;
pub use transition::Transition;

#[cfg(feature = "gpu")]
pub use gpu::{GpuOrb, GpuRenderer};

#[cfg(feature = "gpu")]
pub use transitions::orb_dissolve::OrbDissolve;

#[cfg(feature = "gpu")]
pub use transitions::aqua_dissolve::AquaDissolve;

/// Yield the normalized time `t` for each frame of a clip.
///
/// Produces `frames` values evenly spaced over the closed interval `[0.0, 1.0]`:
/// the first frame is `t = 0.0` (pure `from`) and the last is `t = 1.0`
/// (pure `to`). For `frames <= 1` it yields a single `0.0`.
pub fn timeline(frames: u32) -> impl Iterator<Item = f32> {
    (0..frames.max(1)).map(move |i| {
        if frames <= 1 {
            0.0
        } else {
            i as f32 / (frames - 1) as f32
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_spans_zero_to_one() {
        let ts: Vec<f32> = timeline(5).collect();
        assert_eq!(ts.len(), 5);
        assert_eq!(ts.first(), Some(&0.0));
        assert_eq!(ts.last(), Some(&1.0));
    }

    #[test]
    fn by_name_finds_crossfade() {
        assert!(by_name("crossfade").is_some());
        assert!(by_name("nope").is_none());
    }
}
