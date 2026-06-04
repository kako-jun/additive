//! The ADDITIVE-13 generator contract — source *synthesis*, not transition.
//!
//! A [`Generator`] is the second kind of additive (see [`crate::additive`]). Where
//! a [`crate::Transition`] dissolves a `from` image into a `to` image, a generator
//! *produces* a frame: from a single source image (#19 fake-nyaa — a cat photo →
//! a tileable coat-pattern plane), from a text/parameter input (#20 typewriter
//! parts), or from nothing at all (#21 golden-ratio guide — a pure reference
//! overlay). None of those fit the `(from, to, t)` shape, which is exactly why
//! they live behind their own trait.

use image::RgbaImage;

use crate::additive::Additive;

/// An additive that *synthesizes* a frame rather than transitioning between two
/// images.
///
/// `inputs` carries the zero or more source images the effect draws on: a
/// zero-input generator (a pure reference pattern) ignores it; a single-input
/// generator uses `inputs[0]`. Per-effect knobs (a typewriter's string, a
/// guide's ratio) are threaded by the caller through concrete `*_cfg` methods,
/// exactly as [`crate::Transition`] does with `OrbConfig` — the trait carries
/// only the uniform contract so the catalogue can drive any generator the same
/// way.
pub trait Generator: Additive {
    /// Render the generator's frame at normalized time `t` into a
    /// `width`×`height` canvas, drawing on zero or more source `inputs`.
    ///
    /// `t` is clamped to `0.0..=1.0` by the caller; static generators ignore it.
    fn render(&self, width: u32, height: u32, t: f32, inputs: &[RgbaImage]) -> RgbaImage;

    /// WGSL fragment-shader body for the production (wgpu) render path, when the
    /// generator has one. Defaults to `None` so CPU-only generators (e.g. a
    /// tiny-skia guide overlay) are not forced to carry a shader they do not use.
    #[cfg(feature = "gpu")]
    fn shader_wgsl(&self) -> Option<&'static str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::additive::AdditiveItem;
    use image::Rgba;

    /// A trivial zero-input generator: fills the whole canvas with one color. It
    /// exists to prove the [`Generator`] contract and the [`AdditiveItem`] union
    /// path compile and run end-to-end *before* the real generators (#19/#20/#21)
    /// land — so the abstraction is type-checked, not just sketched.
    struct Fill([u8; 4]);

    impl Additive for Fill {
        fn designation(&self) -> &'static str {
            "No.test"
        }
        fn name(&self) -> &'static str {
            "fill"
        }
        fn description(&self) -> &'static str {
            "Zero-input solid fill (test fixture)."
        }
    }

    impl Generator for Fill {
        fn render(&self, width: u32, height: u32, _t: f32, _inputs: &[RgbaImage]) -> RgbaImage {
            RgbaImage::from_pixel(width, height, Rgba(self.0))
        }
    }

    #[test]
    fn generator_renders_without_inputs() {
        let g = Fill([10, 20, 30, 255]);
        let img = g.render(4, 3, 0.5, &[]);
        assert_eq!(img.dimensions(), (4, 3));
        assert_eq!(img.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert_eq!(img.get_pixel(3, 2).0, [10, 20, 30, 255]);
    }

    #[test]
    fn generator_fits_the_additive_union() {
        let item = AdditiveItem::Generator(Box::new(Fill([0, 0, 0, 255])));
        // The shared identity is reachable through the union without matching on
        // the kind — this is what `--list` / the web picker rely on.
        assert_eq!(item.name(), "fill");
        assert_eq!(item.designation(), "No.test");
        assert_eq!(
            item.as_additive().description(),
            "Zero-input solid fill (test fixture)."
        );
    }
}
