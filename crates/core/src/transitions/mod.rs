//! Built-in additives (transition implementations).
//!
//! `No.0` crossfade is the reference baseline. `No.13` orb-dissolve — the
//! flagship that reuses `orber-core` to break `from` into drifting orbs and
//! reveal `to` beneath — lands in issue #2. `No.14` aqua-dissolve dissolves the
//! from→to seam with the shared `aquarelle` spiral bleed (watercolor にじみ, #28).

pub mod crossfade;
pub mod orb_dissolve;

#[cfg(feature = "gpu")]
pub mod aqua_dissolve;
