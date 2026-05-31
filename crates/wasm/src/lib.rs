//! Browser bindings for additive-core.
//!
//! For now this exposes the **CPU reference** renderer so the web GUI has a
//! working preview from day one. The fast path is wgpu / WebGPU (#1, #4); when it
//! lands, the same WGSL renders here and in the CLI, for identical output.

use additive_core::{all, by_name};
use image::RgbaImage;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Render a single transition frame.
///
/// `from_rgba` / `to_rgba` are straight-alpha RGBA byte buffers of exactly
/// `width * height * 4` bytes. Returns the rendered frame as RGBA bytes.
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
    let tr = by_name(name).ok_or_else(|| JsValue::from_str("unknown transition"))?;
    Ok(tr.render_cpu(&from, &to, t).into_raw())
}

/// Newline-separated names of all built-in transitions.
#[wasm_bindgen]
pub fn transitions() -> String {
    all()
        .iter()
        .map(|t| t.name())
        .collect::<Vec<_>>()
        .join("\n")
}
