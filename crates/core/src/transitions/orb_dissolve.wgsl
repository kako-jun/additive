// No.13 — orb-dissolve (production WGSL).
//
// Composites, per fragment, in raw sRGB byte space (textures bound as
// Rgba8Unorm, NOT *Srgb — same contract as crossfade.wgsl):
//
//   1. base   = `to`                              (opaque background)
//   2. base    = mix(base, from, 1 - t)           (`from` fades out over the clip)
//   3. for each live orb: blend its color over base by a soft radial falloff
//      scaled by the orb's envelope alpha                (the drifting orb field)
//
// Strict per-pixel parity with the CPU/tiny-skia path is NOT a goal here (orber
// itself split over exactly that mismatch); the falloff below is a simple smooth
// disc that reads as an orb, not a tiny-skia gradient clone.
//
// Binding contract (see `gpu.rs` — orb-dissolve extends crossfade's bindings):
//   group(0) binding(0): from texture (texture_2d<f32>)
//   group(0) binding(1): to   texture (texture_2d<f32>)
//   group(0) binding(2): sampler (non-filtering)
//   group(0) binding(3): uniform Params { t, orb_count, _pad0, _pad1 }
//   group(0) binding(4): uniform Orbs { orbs: array<Orb, MAX_ORBS> }

const MAX_ORBS: u32 = 16u;

struct Params {
    t: f32,
    orb_count: f32, // number of live orbs (as f32 to keep 16-byte alignment simple)
    _pad0: f32,
    _pad1: f32,
};

// Each orb: pos (xy, normalized UV), radius (normalized by min axis), alpha
// (envelope opacity), color (rgb straight sRGB 0..1). Laid out as two vec4 so the
// std140 array stride is a clean 32 bytes.
struct Orb {
    pos_radius_alpha: vec4<f32>, // x, y, radius, alpha
    color: vec4<f32>,           // r, g, b, _
};

struct Orbs {
    orbs: array<Orb, 16>,
};

@group(0) @binding(0) var from_tex: texture_2d<f32>;
@group(0) @binding(1) var to_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var<uniform> orb_data: Orbs;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var xy = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = xy[vi];
    var out: VsOut;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let from_color = textureSample(from_tex, samp, in.uv);
    let to_color = textureSample(to_tex, samp, in.uv);

    // 1+2: `to` background with `from` fading out.
    var base = mix(to_color.rgb, from_color.rgb, 1.0 - params.t);

    // Aspect-correct distance: radius is normalized by the short axis. We only
    // have UV here, so approximate isotropy by working in UV space directly; the
    // orb reads as an ellipse on non-square frames, which is acceptable for the
    // 叩き台 mechanic (kako-jun tunes the look later).
    let count = u32(params.orb_count + 0.5);
    for (var i: u32 = 0u; i < MAX_ORBS; i = i + 1u) {
        if (i >= count) {
            break;
        }
        let o = orb_data.orbs[i];
        let center = o.pos_radius_alpha.xy;
        let radius = o.pos_radius_alpha.z;
        let env = o.pos_radius_alpha.w;
        if (radius <= 0.0 || env <= 0.0) {
            continue;
        }
        let d = distance(in.uv, center);
        // Soft disc: full inside 40% of radius, smooth fade to the rim.
        let inner = radius * 0.4;
        let falloff = 1.0 - smoothstep(inner, radius, d);
        let a = clamp(falloff * env, 0.0, 1.0);
        base = mix(base, o.color.rgb, a);
    }

    return vec4<f32>(base, 1.0);
}
