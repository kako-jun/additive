//! The ADDITIVE-13 catalogue contract.
//!
//! Every entry in the series is an [`Additive`] — it carries an E-number style
//! designation (a nod to パトレイバー 廃棄物13号 and food-additive E-numbers) and
//! a stable kebab-case name used on the CLI and in the web GUI.
//!
//! An additive is rendered as **either** a [`Transition`] — a two-image time
//! function `(from, to, t) -> frame` (No.0, No.13, …) — **or** a [`Generator`] —
//! source synthesis from zero or one image (the parts/material generators
//! #19/#20/#21). The two render contracts are genuinely different shapes, so the
//! registry is a [`AdditiveItem`] union rather than a single `dyn` trait; the
//! shared [`Additive`] identity is what the catalogue (`--list`, the web picker)
//! iterates uniformly.

use crate::generator::Generator;
use crate::transition::Transition;
use crate::transitions::crossfade::Crossfade;
#[cfg(feature = "gpu")]
use crate::transitions::orb_dissolve::OrbDissolve;

/// Shared identity of every entry in the series — the "添加物 No.N".
///
/// Both [`Transition`]s and [`Generator`]s are `Additive`s. This supertrait
/// holds only the catalogue metadata; the render contract lives on the kind-
/// specific subtrait so a generator never has to pretend it takes a `to` image.
pub trait Additive {
    /// E-number style designation, e.g. `"No.13"`. The flagship orb-dissolve is
    /// `No.13`.
    fn designation(&self) -> &'static str;

    /// Stable kebab-case identifier, e.g. `"orb-dissolve"`.
    fn name(&self) -> &'static str;

    /// One-line human description.
    fn description(&self) -> &'static str;
}

/// One registered additive: either a [`Transition`] or a [`Generator`].
///
/// The two render contracts are deliberately distinct — [`Transition`] takes a
/// `from`/`to` pair, [`Generator`] does not — so the catalogue is a union and
/// callers match on the kind to drive the matching render path. Listing and
/// lookup go through the shared [`Additive`] accessors, which work without
/// knowing the kind.
pub enum AdditiveItem {
    /// A two-image time function `(from, to, t) -> frame` (No.0, No.13, …).
    Transition(Box<dyn Transition>),
    /// Source synthesis from zero or one image (#19 fake-nyaa / #20 typewriter /
    /// #21 golden-ratio guide). The first generator effect populates this; until
    /// one lands, no built-in constructs it.
    Generator(Box<dyn Generator>),
}

impl AdditiveItem {
    /// Borrow the entry as its shared [`Additive`] identity, for catalogue
    /// listing: designation / name / description regardless of the kind.
    ///
    /// Each arm upcasts `&dyn Transition` / `&dyn Generator` to the supertrait
    /// `&dyn Additive` — trait upcasting, stable since Rust 1.86 (this crate's
    /// `rust-version` is 1.87). Lowering MSRV below 1.86 would break this line.
    pub fn as_additive(&self) -> &dyn Additive {
        match self {
            AdditiveItem::Transition(t) => t.as_ref(),
            AdditiveItem::Generator(g) => g.as_ref(),
        }
    }

    /// E-number designation (shared identity).
    pub fn designation(&self) -> &'static str {
        self.as_additive().designation()
    }

    /// Stable kebab-case name (shared identity).
    pub fn name(&self) -> &'static str {
        self.as_additive().name()
    }

    /// One-line description (shared identity).
    pub fn description(&self) -> &'static str {
        self.as_additive().description()
    }
}

/// All built-in additives, in designation order.
///
/// No.13 orb-dissolve relies on the `orber-core` orb engine, which is pulled in
/// only under the `gpu` feature, so it is registered only in that build. (The
/// wasm / no-gpu build exposes just No.0 crossfade until the browser path lands.)
pub fn all() -> Vec<AdditiveItem> {
    let items: Vec<AdditiveItem> = vec![
        AdditiveItem::Transition(Box::new(Crossfade)),
        #[cfg(feature = "gpu")]
        AdditiveItem::Transition(Box::new(OrbDissolve)),
    ];
    items
}

/// Look up a built-in additive by its kebab-case `name`.
pub fn by_name(name: &str) -> Option<AdditiveItem> {
    all().into_iter().find(|item| item.name() == name)
}
