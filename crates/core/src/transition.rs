//! The ADDITIVE-13 transition contract.

use image::RgbaImage;

use crate::transitions::crossfade::Crossfade;

/// A named transition effect — one "additive" in the series.
///
/// Each additive carries an E-number style [`designation`](Transition::designation)
/// (a nod to パトレイバー 廃棄物13号 and food-additive E-numbers) and a stable
/// kebab-case [`name`](Transition::name) used on the CLI and in the web GUI.
pub trait Transition {
    /// E-number style designation, e.g. `"No.13"`. The flagship orb-dissolve is
    /// `No.13`.
    fn designation(&self) -> &'static str;

    /// Stable kebab-case identifier, e.g. `"orb-dissolve"`.
    fn name(&self) -> &'static str;

    /// One-line human description.
    fn description(&self) -> &'static str;

    /// Reference (CPU) render of the frame at time `t`.
    ///
    /// `from` and `to` must have identical dimensions; callers resize beforehand.
    /// `t` is clamped to `0.0..=1.0`. This is the parity oracle the wgpu renderer
    /// (#1) is checked against.
    fn render_cpu(&self, from: &RgbaImage, to: &RgbaImage, t: f32) -> RgbaImage;

    /// WGSL fragment-shader body for the production (wgpu) render path.
    ///
    /// The shader is run by [`crate::gpu::GpuRenderer`] against a full-screen
    /// triangle. It must define a fragment entry point `fs_main` that samples the
    /// bound `from`/`to` textures and the `t` uniform, mixing them in **sRGB byte
    /// space without gamma conversion** so the result matches [`render_cpu`]
    /// channel-for-channel (see [`crate::gpu`] for the binding contract).
    #[cfg(feature = "gpu")]
    fn shader_wgsl(&self) -> &'static str;
}

/// All built-in transitions, in designation order.
pub fn all() -> Vec<Box<dyn Transition>> {
    vec![Box::new(Crossfade)]
}

/// Look up a built-in transition by its kebab-case `name`.
pub fn by_name(name: &str) -> Option<Box<dyn Transition>> {
    all().into_iter().find(|t| t.name() == name)
}
