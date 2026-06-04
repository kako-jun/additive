//! Browser bindings for additive-core.
//!
//! For now this exposes the **CPU reference** renderer so the web GUI has a
//! working preview from day one. The fast path is wgpu / WebGPU (#1, #4); when it
//! lands, the same WGSL renders here and in the CLI, for identical output.

use additive_core::{all, by_name, AdditiveItem};
use image::RgbaImage;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Render a single additive frame.
///
/// `from_rgba` / `to_rgba` are straight-alpha RGBA byte buffers of exactly
/// `width * height * 4` bytes. Returns the rendered frame as RGBA bytes. For a
/// transition both buffers are mixed by `t`; for a generator they are passed as
/// the available source `inputs` (a zero-input generator ignores them).
#[wasm_bindgen]
pub fn render_frame(
    name: &str,
    width: u32,
    height: u32,
    from_rgba: &[u8],
    to_rgba: &[u8],
    t: f32,
) -> Result<Vec<u8>, JsValue> {
    let expected = (width as usize) * (height as usize) * 4;
    if from_rgba.len() != expected || to_rgba.len() != expected {
        return Err(JsValue::from_str(
            "from/to byte length must equal width*height*4",
        ));
    }
    let from = RgbaImage::from_raw(width, height, from_rgba.to_vec())
        .ok_or_else(|| JsValue::from_str("invalid from buffer"))?;
    let to = RgbaImage::from_raw(width, height, to_rgba.to_vec())
        .ok_or_else(|| JsValue::from_str("invalid to buffer"))?;
    let frame = match by_name(name).ok_or_else(|| JsValue::from_str("unknown additive"))? {
        AdditiveItem::Transition(tr) => tr.render_cpu(&from, &to, t),
        // Generators (#19/#20/#21) synthesize from zero/one image with per-effect
        // inputs that this two-image preview entry point can't express yet. No
        // built-in generator exists, so this is unreachable today — but, like the
        // CLI, fail loudly rather than guess the inputs. Wiring lands with the
        // first real generator (it will likely revise this signature; see #4/#5).
        AdditiveItem::Generator(g) => {
            return Err(JsValue::from_str(&format!(
                "'{}' is a generator; the generator render path is not wired into the wasm preview yet (see #19/#20/#21)",
                g.name()
            )));
        }
    };
    Ok(frame.into_raw())
}

/// Newline-separated names of all built-in additives.
#[wasm_bindgen]
pub fn transitions() -> String {
    all()
        .iter()
        .map(|item| item.name())
        .collect::<Vec<_>>()
        .join("\n")
}
