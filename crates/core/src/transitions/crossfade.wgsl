// No.0 — crossfade (production WGSL).
//
// Mixes `from` and `to` linearly by `t`, per channel including alpha, in the raw
// sRGB byte space the CPU oracle uses. The textures are bound as `Rgba8Unorm`
// (NOT `*Srgb`), so sampling yields the stored bytes / 255 with no gamma
// conversion; `mix` here therefore matches the CPU `f·(1−t) + g·t` exactly.
//
// Binding contract (see `gpu.rs`):
//   group(0) binding(0): from texture (texture_2d<f32>)
//   group(0) binding(1): to   texture (texture_2d<f32>)
//   group(0) binding(2): sampler (non-filtering; we sample at pixel centers)
//   group(0) binding(3): uniform { t: f32 }

struct Params {
    t: f32,
};

@group(0) @binding(0) var from_tex: texture_2d<f32>;
@group(0) @binding(1) var to_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Full-screen triangle generated from the vertex index — no vertex buffer.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Clip-space positions covering the screen with a single oversized triangle.
    var xy = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = xy[vi];
    var out: VsOut;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    // Map clip space to UV. Clip y is up; texture/render-target v is down.
    out.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let from_color = textureSample(from_tex, samp, in.uv);
    let to_color = textureSample(to_tex, samp, in.uv);
    return mix(from_color, to_color, params.t);
}
