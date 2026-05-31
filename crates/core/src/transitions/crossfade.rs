//! No.0 — crossfade.

use image::RgbaImage;

use crate::transition::Transition;

/// **No.0 — crossfade.** A plain linear cross-dissolve
/// (`out = from·(1−t) + to·t`, per channel including alpha). The humble baseline
/// of the series; it also doubles as the parity oracle wgpu additives are
/// verified against.
pub struct Crossfade;

impl Transition for Crossfade {
    fn designation(&self) -> &'static str {
        "No.0"
    }

    fn name(&self) -> &'static str {
        "crossfade"
    }

    fn description(&self) -> &'static str {
        "Linear cross-dissolve between the two images."
    }

    fn render_cpu(&self, from: &RgbaImage, to: &RgbaImage, t: f32) -> RgbaImage {
        debug_assert_eq!(
            from.dimensions(),
            to.dimensions(),
            "from and to must share dimensions"
        );
        let t = t.clamp(0.0, 1.0);
        let (w, h) = from.dimensions();
        let mut out = RgbaImage::new(w, h);
        for (o, (f, g)) in out.pixels_mut().zip(from.pixels().zip(to.pixels())) {
            for c in 0..4 {
                let v = f.0[c] as f32 * (1.0 - t) + g.0[c] as f32 * t;
                o.0[c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(px))
    }

    #[test]
    fn endpoints_are_pure() {
        let a = solid(2, 2, [10, 20, 30, 255]);
        let b = solid(2, 2, [200, 100, 0, 255]);
        let cf = Crossfade;
        assert_eq!(
            cf.render_cpu(&a, &b, 0.0).get_pixel(0, 0).0,
            [10, 20, 30, 255]
        );
        assert_eq!(
            cf.render_cpu(&a, &b, 1.0).get_pixel(0, 0).0,
            [200, 100, 0, 255]
        );
    }

    #[test]
    fn midpoint_is_average() {
        let a = solid(1, 1, [0, 0, 0, 0]);
        let b = solid(1, 1, [100, 100, 100, 100]);
        let cf = Crossfade;
        assert_eq!(
            cf.render_cpu(&a, &b, 0.5).get_pixel(0, 0).0,
            [50, 50, 50, 50]
        );
    }
}
