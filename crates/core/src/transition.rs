//! The ADDITIVE-13 transition contract — the two-image time function.

use image::RgbaImage;

use crate::additive::Additive;

/// A two-image transition — one "additive" in the series whose render shape is
/// `(from, to, t) -> frame`.
///
/// A transition is an [`Additive`] (it carries the shared
/// designation/name/description identity) that additionally maps a `from`/`to`
/// pair and a normalized time `t` to a frame. Source-synthesis effects that have
/// no meaningful `to` use [`crate::Generator`] instead — they are not forced
/// through this contract.
pub trait Transition: Additive {
    /// Reference (CPU) render of the frame at time `t`.
    ///
    /// `from` and `to` must have identical dimensions; callers resize beforehand.
    /// `t` is clamped to `0.0..=1.0`. This is the parity oracle the wgpu renderer
    /// (#1) is checked against.
    fn render_cpu(&self, from: &RgbaImage, to: &RgbaImage, t: f32) -> RgbaImage;

    /// WGSL fragment-shader body for the production (wgpu) render path, when the
    /// transition has one.
    ///
    /// The shader is run by [`crate::gpu::GpuRenderer`] against a full-screen
    /// triangle. It must define a fragment entry point `fs_main` that samples the
    /// bound `from`/`to` textures and the `t` uniform, mixing them in **sRGB byte
    /// space without gamma conversion** so the result matches [`render_cpu`]
    /// channel-for-channel (see [`crate::gpu`] for the binding contract).
    ///
    /// Defaults to `None` so a CPU-only transition (tiny-skia only) is not forced
    /// to carry a GPU shader it does not have; the CLI errors clearly if a
    /// shaderless transition is asked to render on the GPU path.
    #[cfg(feature = "gpu")]
    fn shader_wgsl(&self) -> Option<&'static str> {
        None
    }
}
