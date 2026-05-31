//! ADDITIVE-13 — pure transition-rendering core.
//!
//! ADDITIVE-13 is a *series of image transitions* ("additives"). Every additive
//! is a transition between two same-sized images, expressed as a single pure
//! function of normalized time `t` in `0.0..=1.0`:
//!
//! ```text
//! (from, to, t) -> RGBA frame
//! ```
//!
//! See [`Transition`] for the contract. The renderer in this crate is the
//! **reference** CPU path; the production renderer is wgpu (Rust + WGSL), shared
//! by the native CLI and the browser so both produce identical output (issue #1).

#[cfg(feature = "gpu")]
pub mod gpu;
pub mod transition;
pub mod transitions;

#[cfg(feature = "gpu")]
pub use gpu::{GpuOrb, GpuRenderer};
pub use transition::{all, by_name, Transition};

#[cfg(feature = "gpu")]
pub use transitions::orb_dissolve::OrbDissolve;

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
