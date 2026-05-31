// No.13 — orb-dissolve (production WGSL), full-occlusion redefinition.
//
// Composites, per fragment, in raw sRGB byte space (textures bound as
// Rgba8Unorm, NOT *Srgb — same contract as crossfade.wgsl):
//
//   1. base = mix(to, from, base_from_weight(t))   (HARD `from→to` swap at t≈0.5,
//      with a ±0.05 micro cross-fade — see `OrbDissolve::base_from_weight`)
//   2. for each live orb: blend its color over base by a soft radial falloff
//      scaled by the orb's envelope alpha            (the gap-free orb curtain)
//
// At the occlusion plateau (t≈0.5) the orbs are simultaneously largest and fully
// opaque, so the base swap underneath is invisible: the frame becomes independent
// of both `from` and `to` (full occlusion). The mechanism — base swap + orb
// falloff (inner core at 0.7·radius, aspect-corrected isotropic distance) — is
// kept in lockstep with the CPU oracle (`render_cpu_cfg`) so both renderers reach
// the same full occlusion, even without strict per-pixel parity.
//
// Binding contract (see `gpu.rs` — orb-dissolve extends crossfade's bindings):
//   group(0) binding(0): from texture (texture_2d<f32>)
//   group(0) binding(1): to   texture (texture_2d<f32>)
//   group(0) binding(2): sampler (non-filtering)
//   group(0) binding(3): uniform Params { t, orb_count, aspect_x, aspect_y }
//   group(0) binding(4): uniform Orbs { orbs: array<Orb, MAX_ORBS> }

const MAX_ORBS: u32 = 128u;

struct Params {
    t: f32,
    orb_count: f32, // number of live orbs (as f32 to keep 16-byte alignment simple)
    aspect_x: f32,  // width  / min(width, height) — UV→isotropic x scale
    aspect_y: f32,  // height / min(width, height) — UV→isotropic y scale
};

// Each orb: pos (xy, normalized UV), radius (normalized by min axis), alpha
// (envelope opacity), color (rgb straight sRGB 0..1). Laid out as two vec4 so the
// std140 array stride is a clean 32 bytes.
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

// from-weight of the base layer: a HARD swap at t=0.5 with a ±0.05 micro
// cross-fade. Mirrors `OrbDissolve::base_from_weight` on the CPU exactly.
fn base_from_weight(t: f32) -> f32 {
    let half = 0.05;
    let lo = 0.5 - half;
    let hi = 0.5 + half;
    if (t <= lo) {
        return 1.0;
    } else if (t >= hi) {
        return 0.0;
    }
    return 1.0 - (t - lo) / (hi - lo);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let from_color = textureSample(from_tex, samp, in.uv);
    let to_color = textureSample(to_tex, samp, in.uv);

    // 1: base = from→to HARD swap (orb curtain hides it across the plateau).
    let fw = base_from_weight(params.t);
    var base = mix(to_color.rgb, from_color.rgb, fw);

    // 2: gap-free orb curtain. Distance is aspect-corrected into shorter-axis
    // units so radii read isotropically on non-square frames (matches the CPU
    // oracle). The opaque core spans 0.7·radius for hole-free coverage.
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
        // Toroidal (wrapped) delta: the orb field tiles [0,1]^2 and the conveyor
        // wraps, so an orb near an edge also covers the opposite edge (no seam).
        let wx = (in.uv.x - center.x) - floor((in.uv.x - center.x) + 0.5);
        let wy = (in.uv.y - center.y) - floor((in.uv.y - center.y) + 0.5);
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
