// No.13 — orb-dissolve (production WGSL), conveyor sweep-wipe.
//
// Composites, per fragment, in raw sRGB byte space (textures bound as
// Rgba8Unorm, NOT *Srgb — same contract as crossfade.wgsl):
//
//   1. base = directional from→to SWEEP. The wipe front travels along the flow
//      axis; ahead of the front the base is `from`, behind it `to`. The seam is a
//      near-hard step (the orb band hides it).
//   2. for each live orb: blend its color over base by a soft radial falloff
//      scaled by the orb's band alpha     (the sweeping orb band that hides the seam)
//
// The orbs are concentrated in a band around the front, so the from→to seam in the
// base is always under opaque orbs — the boundary is never directly visible. The
// band sweeps from the entry edge to the exit edge as t:0→1 (a wipe), it does NOT
// cover the whole frame at once. The disc falloff (inner core at 0.7·radius,
// aspect-corrected isotropic distance) is kept in lockstep with the CPU oracle
// (`render_cpu_cfg`).
//
// Binding contract (see `gpu.rs` — orb-dissolve extends crossfade's bindings):
//   group(0) binding(0): from texture (texture_2d<f32>)
//   group(0) binding(1): to   texture (texture_2d<f32>)
//   group(0) binding(2): sampler (non-filtering)
//   group(0) binding(3): uniform Params { t, orb_count, aspect_x, aspect_y,
//                                         front, dir_code, _pad0, _pad1 }
//   group(0) binding(4): uniform Orbs { orbs: array<Orb, MAX_ORBS> }

const MAX_ORBS: u32 = 128u;

struct Params {
    t: f32,
    orb_count: f32, // number of live orbs (as f32 to keep alignment simple)
    aspect_x: f32,  // width  / min(width, height) — UV→isotropic x scale
    aspect_y: f32,  // height / min(width, height) — UV→isotropic y scale
    front: f32,     // wipe-front position along flow axis (positive-axis sense)
    dir_code: f32,  // 0 lr, 1 rl, 2 tb, 3 bt
    _pad0: f32,
    _pad1: f32,
};

// Each orb: pos (xy, normalized UV), radius (normalized by min axis), alpha
// (band opacity), color (rgb straight sRGB 0..1). Two vec4 ⇒ 32-byte stride.
struct Orb {
    pos_radius_alpha: vec4<f32>, // x, y, radius, alpha
    color: vec4<f32>,           // r, g, b, _
};

struct Orbs {
    orbs: array<Orb, 128>,
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

// from-weight of the base layer at flow-axis coordinate `u`: a directional sweep.
// 1.0 ahead of the front (still `from`), 0.0 behind it (already `to`). Mirrors
// `OrbDissolve::base_from_weight_at`. A tiny SEAM_SOFT ramp avoids aliasing.
fn base_from_weight_at(u: f32) -> f32 {
    let seam_soft = 0.012;
    let dir = params.dir_code;
    let is_negative = (dir > 0.5 && dir < 1.5) || (dir > 2.5); // rl or bt
    var behind: f32;
    if (is_negative) {
        behind = u - (1.0 - params.front);
    } else {
        behind = params.front - u;
    }
    // behind > 0 ⇒ swept ⇒ to (weight 0); behind < 0 ⇒ ahead ⇒ from (weight 1).
    return smoothstep(0.0, seam_soft, -behind);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let from_color = textureSample(from_tex, samp, in.uv);
    let to_color = textureSample(to_tex, samp, in.uv);

    // Flow-axis coordinate for this pixel (lr/rl ⇒ x, tb/bt ⇒ y).
    let dir = params.dir_code;
    let is_horizontal = (dir < 0.5) || (dir > 0.5 && dir < 1.5); // lr or rl
    var u: f32;
    if (is_horizontal) {
        u = in.uv.x;
    } else {
        u = in.uv.y;
    }

    // 1: base = directional from→to sweep (orb band hides the seam).
    let fw = base_from_weight_at(u);
    var base = mix(to_color.rgb, from_color.rgb, fw);

    // 2: sweeping orb band. Distance is aspect-corrected into shorter-axis units
    // so radii read isotropically (matches the CPU oracle). Wrap only on the cross
    // axis (the band tiles the cross-span; the flow axis is open).
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
        let rawx = in.uv.x - center.x;
        let rawy = in.uv.y - center.y;
        var wx: f32;
        var wy: f32;
        if (is_horizontal) {
            // flow = x (open), cross = y (wrapped).
            wx = rawx;
            wy = rawy - floor(rawy + 0.5);
        } else {
            // flow = y (open), cross = x (wrapped).
            wx = rawx - floor(rawx + 0.5);
            wy = rawy;
        }
        let dx = wx * params.aspect_x;
        let dy = wy * params.aspect_y;
        let d = sqrt(dx * dx + dy * dy);
        let inner = radius * 0.7;
        let falloff = 1.0 - smoothstep(inner, radius, d);
        let a = clamp(falloff * env, 0.0, 1.0);
        base = mix(base, o.color.rgb, a);
    }

    return vec4<f32>(base, 1.0);
}
