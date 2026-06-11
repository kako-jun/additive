// No.14 — aqua-dissolve (production WGSL), watercolor seam dissolve (にじみ).
//
// Composites, per fragment, in raw sRGB byte space (textures bound as
// Rgba8Unorm, NOT *Srgb — same contract as crossfade.wgsl / orb_dissolve.wgsl).
//
// A directional from→to SWEEP: the wipe front travels along the flow axis, `from`
// ahead of it, `to` behind it. The from→to boundary is NOT a hard line — it is run
// through the shared aquarelle spiral bleed (`blurred_coverage`, 48-tap golden-angle
// spiral, per-pixel hash dither) so it wicks/feathers like wet pigment on paper. A
// faint saturated tint (`aqua_character` halo) is added at the wet rim.
//
// === Host-supplied symbols the shared bleed fragment requires ===
//
// `aquarelle::AQUA_BLEED_WGSL` is substituted at the shared-bleed marker line
// below (a lone `//!`-prefixed comment). It calls these symbols, which MUST be
// defined here BEFORE that marker:
//   const TAU: f32;
//   fn hash21(p: vec2<f32>) -> f32;
//   fn clampf(x: f32, a: f32, b: f32) -> f32;
//   fn coverage_at(style_bit, sample_px, cx, cy, radius, blur, opacity, angle) -> vec2<f32>;
//
// Our `coverage_at` returns the **`to`-region silhouette** at the spiral tap point:
// straight-alpha 1.0 where the tap is behind the wipe front (already `to`), 0.0
// ahead of it, with a SEAM_SOFT ramp across the front. `blurred_coverage` averages
// that over the spiral disc, so the boundary feathers. The flow-axis sweep state the
// silhouette needs (front position, direction) is threaded through the otherwise-
// unused `coverage_at` args: `cx = front`, `cy = dir_code`.
//
// Binding contract (group(0)): same as orb-dissolve's first four bindings, with the
// orb-array uniform (binding 4) repurposed as the aqua sweep uniform.

const TAU: f32 = 6.28318530718;

// Half-width of the unbled seam ramp (normalized flow units). The visible width of
// the watercolor edge comes from the spiral bleed; this just avoids a 1-px alias.
const SEAM_SOFT: f32 = 0.02;

struct Params {
    t: f32,
    _pad_a: f32,
    aspect_x: f32, // width  / min(width, height) — UV→isotropic x scale
    aspect_y: f32, // height / min(width, height) — UV→isotropic y scale
    front: f32,    // wipe-front position along flow axis (positive-axis sense)
    dir_code: f32, // 0 lr, 1 rl, 2 tb, 3 bt
    _pad_b: f32,
    _pad_c: f32,
};

// Aqua sweep uniform (binding 4): the spiral-bleed knobs. `bleed` is the disc
// radius in shorter-axis-normalized units; `halo`/`bloom` drive `aqua_character`;
// `seed` is the deterministic dither phase. Padded to a vec4 pair (32 bytes).
struct AquaSweep {
    bleed: f32,
    halo: f32,
    bloom: f32,
    seed: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
};

@group(0) @binding(0) var from_tex: texture_2d<f32>;
@group(0) @binding(1) var to_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var<uniform> aqua: AquaSweep;

// --- Host symbols the shared bleed fragment requires --------------------------

fn hash21(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

fn clampf(x: f32, a: f32, b: f32) -> f32 {
    return min(max(x, a), b);
}

// `to`-region silhouette at a spiral tap. We work in NORMALIZED flow-axis space:
// the spiral disc is built in normalized UV (the host scales blur_px into UV before
// calling `blurred_coverage`), so `sample_px` here is a normalized UV point.
//   cx = front (positive-axis sense),  cy = dir_code (0 lr / 1 rl / 2 tb / 3 bt).
// Returns (straight_alpha = to-weight, rgb_scale = 1.0). The other args (radius /
// blur / opacity / angle) are unused for this silhouette.
fn coverage_at(
    style_bit: f32,
    sample_px: vec2<f32>,
    cx: f32,
    cy: f32,
    radius: f32,
    blur: f32,
    opacity: f32,
    angle: f32,
) -> vec2<f32> {
    let front = cx;
    let dir = cy;
    let is_horizontal = (dir < 0.5) || (dir > 0.5 && dir < 1.5); // lr or rl
    let is_negative = (dir > 0.5 && dir < 1.5) || (dir > 2.5);   // rl or bt
    var u: f32;
    if (is_horizontal) {
        u = sample_px.x;
    } else {
        u = sample_px.y;
    }
    var behind: f32;
    if (is_negative) {
        behind = u - (1.0 - front);
    } else {
        behind = front - u;
    }
    // behind > 0 ⇒ swept ⇒ to (weight 1); behind < 0 ⇒ ahead ⇒ from (weight 0).
    return vec2<f32>(smoothstep(0.0, SEAM_SOFT, behind), 1.0);
}

//!AQUA_BLEED_SHARED

// --- Vertex + fragment --------------------------------------------------------

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

    // The spiral disc is isotropic in shorter-axis units. The silhouette only
    // varies along the flow axis, so cross-axis anisotropy is invisible; we build
    // the disc in normalized UV using the flow-axis scale. `bleed` is in
    // shorter-axis-normalized units, so divide by the flow axis's aspect to get a
    // UV radius (aspect_x = width/short means a shorter-axis unit is 1/aspect_x of
    // the UV width).
    let dir = params.dir_code;
    let is_horizontal = (dir < 0.5) || (dir > 0.5 && dir < 1.5);
    var flow_aspect: f32;
    if (is_horizontal) {
        flow_aspect = params.aspect_x;
    } else {
        flow_aspect = params.aspect_y;
    }
    let blur_uv = aqua.bleed / max(flow_aspect, 1e-4);

    // Spiral-averaged `to` weight (the feathered, wicking boundary). Thread the
    // sweep state through coverage_at's unused args: cx=front, cy=dir_code. radius/
    // blur/opacity/angle/style_bit are unused by our silhouette (pass 0/1).
    let cov = blurred_coverage(
        0.0,        // style_bit (unused)
        in.uv,      // sample_px (normalized UV)
        params.front, // cx = front
        dir,        // cy = dir_code
        1.0,        // radius (unused)
        0.0,        // blur (unused)
        1.0,        // opacity (unused)
        0.0,        // angle (unused)
        blur_uv,    // blur_px = disc radius in UV
        aqua.seed,  // per-pixel dither seed
        vec2<f32>(0.0, 0.0), // bias_px (symmetric)
    );
    let to_w = clampf(cov.x, 0.0, 1.0);

    // Base mix: from ahead → to behind.
    var rgb = mix(from_color.rgb, to_color.rgb, to_w);

    // Wet-rim character: tint only the wet edge (where to_w is mid), never the flat
    // from/to interiors. `aqua_character` keys its halo off LOW coverage alpha, so
    // we feed it the rim as "center" (cov_a = rim, high at the boundary) and blend
    // the result in proportional to `rim` so the flats stay exactly from/to. At the
    // endpoints to_w is flat 0/1, rim is 0, and the base mix is returned untouched.
    if (aqua.halo > 0.0 || aqua.bloom > 0.0) {
        let rim = 1.0 - abs(2.0 * to_w - 1.0);
        if (rim > 0.0) {
            let tinted = aqua_character(rgb, rim, aqua.bloom, aqua.halo);
            rgb = mix(rgb, tinted, rim);
        }
    }

    return vec4<f32>(rgb, 1.0);
}
