// The display pass (display.rs, decision 0019): each pixel's light (lighting.wgsl, which comes
// first in this shader and declares bindings 1 to 7) to 8-bit sRGB screen pixels. Per pixel:
// the background and the floor grid's planes behind the surface; the surface through the view
// transform by kind of pixel, with the wire's edges over it in their own color; the planes' lines in front of it; the active part's outline over
// that; the running mean of the accumulated samples; then the sRGB encoding and the dither. The
// view transforms are the twins of color.rs.

@group(0) @binding(8) var history: texture_2d<f32>;
// Each pixel's part flags from the geometry pass (decision 0033): bit 0, the active part. Read
// four at a time through a nearest sampler that clamps at the edges.
@group(0) @binding(14) var parts: texture_2d<u32>;
@group(0) @binding(15) var parts_gather: sampler;

// The active part's outline (decision 0033, section 5): the orange of Blender's active object,
// #FFAA40, display-linear.
const OUTLINE: vec3<f32> = vec3<f32>(1.0, 0.402, 0.0513);

// The smaller of `near` and the squared distance `d2` (9: none) of each marked pixel among the
// 2 by 2 pixels around frame point `corner`, in the order textureGather returns them.
fn nearest(corner: vec2<f32>, d2: vec4<f32>, near: f32) -> f32 {
    let size = vec2<f32>(textureDimensions(parts));
    let g = textureGather(0, parts, parts_gather, corner / size);
    let d = select(vec4<f32>(9.0), d2, (g & vec4<u32>(1u)) == vec4<u32>(1u));
    return min(near, min(min(d.x, d.y), min(d.z, d.w)));
}

// How much of the active part's outline covers frame pixel `at`: a line about 2 pixels wide
// just outside the marked pixels, from the nearest marked pixel within 2 pixels (full at 1 and
// on the diagonal, half at 2); none on a marked pixel, so the part's own pixels never change.
// Four gathers a quarter turn apart hold the 12 pixels within 2, and 4 at the square root of 5
// that are left out: 5 reads a pixel instead of 13, and the reads are what this costs.
fn outline(at: vec2<i32>) -> f32 {
    let hi = vec2<i32>(textureDimensions(parts)) - 1;
    if (textureLoad(parts, clamp(at, vec2<i32>(0), hi), 0).x & 1u) == 1u {
        return 0.0;
    }
    let p = vec2<f32>(at);
    var near = nearest(p + vec2<f32>(2.0, 1.0), vec4<f32>(2.0, 9.0, 4.0, 1.0), 9.0);
    near = nearest(p + vec2<f32>(0.0, 2.0), vec4<f32>(9.0, 4.0, 1.0, 2.0), near);
    near = nearest(p + vec2<f32>(-1.0, 0.0), vec4<f32>(4.0, 1.0, 2.0, 9.0), near);
    near = nearest(p + vec2<f32>(1.0, -1.0), vec4<f32>(1.0, 2.0, 9.0, 4.0), near);
    return clamp(2.5 - sqrt(near), 0.0, 1.0);
}

fn max3(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b));
}

// Khronos PBR Neutral (color.rs `pbr_neutral`).
fn pbr_neutral(rgb: vec3<f32>) -> vec3<f32> {
    let f90 = 0.04;
    let ks = 0.8 - f90;
    let kd = 0.15;
    var c = max(rgb, vec3<f32>(0.0));
    let x = min(c.r, min(c.g, c.b));
    let offset = select(f90, x - x * x / (4.0 * f90), x < 2.0 * f90);
    c -= offset;
    let peak = max3(c);
    if peak < ks {
        return c;
    }
    let dd = 1.0 - ks;
    let new_peak = 1.0 - dd * dd / (peak + dd - ks);
    let g = 1.0 / (kd * (peak - new_peak) + 1.0);
    return c * (new_peak / peak) * g + new_peak * (1.0 - g);
}

// AgX after Filament's and three.js's operator (color.rs `agx`). WGSL matrices take columns, so Filament's
// numbers go in as written there; the Rec. 709 and 2020 conversions are BT.2087's.
fn agx(rgb: vec3<f32>) -> vec3<f32> {
    let to2020 = mat3x3<f32>(
        vec3<f32>(0.627404, 0.069097, 0.0163916),
        vec3<f32>(0.329282, 0.91954, 0.0880132),
        vec3<f32>(0.0433136, 0.0113612, 0.895595));
    let to709 = mat3x3<f32>(
        vec3<f32>(1.660491, -0.1245505, -0.0181508),
        vec3<f32>(-0.5876411, 1.1328999, -0.1005789),
        vec3<f32>(-0.0728499, -0.0083494, 1.1187297));
    let inset = mat3x3<f32>(
        vec3<f32>(0.856627153315983, 0.137318972929847, 0.11189821299995),
        vec3<f32>(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
        vec3<f32>(0.0482516061458583, 0.101439036467562, 0.811302368396859));
    let outset = mat3x3<f32>(
        vec3<f32>(1.1271005818144368, -0.1413297634984383, -0.14132976349843826),
        vec3<f32>(-0.11060664309660322, 1.1578237022162718, -0.11060664309660294),
        vec3<f32>(-0.01649393871783457, -0.016493938717834257, 1.2519364065950402));
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = inset * (to2020 * max(rgb, vec3<f32>(0.0)));
    v = clamp((log2(max(v, vec3<f32>(1e-10))) - min_ev) / (max_ev - min_ev), vec3<f32>(0.0), vec3<f32>(1.0));
    let x2 = v * v;
    let x4 = x2 * x2;
    v = 15.5 * x4 * x2 - 40.14 * x4 * v + 31.96 * x4 - 6.868 * x2 * v + 0.4298 * x2 + 0.1191 * v - 0.00232;
    v = pow(max(outset * v, vec3<f32>(0.0)), vec3<f32>(2.2));
    return clamp(to709 * v, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn view_transform(c: vec3<f32>) -> vec3<f32> {
    switch f.flags.x {
        case 0u: { return pbr_neutral(c); }
        case 1u: { return agx(c); }
        default: { return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)); }
    }
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// A fixed hash of the pixel (PCG), as a number in 0..1. Integer math, so every GPU agrees.
fn hash(p: vec2<u32>, salt: u32) -> f32 {
    var s = p.x * 1973u + p.y * 9277u + salt * 26699u;
    s = s * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return f32((w >> 22u) ^ w) / 4294967296.0;
}

// How much of a line of cells `g` (in cell units) covers this pixel, about one pixel wide from
// the screen derivative `w`, faded out where lines would crowd closer than four pixels.
fn lines(g: vec2<f32>, w: vec2<f32>) -> f32 {
    let off = abs(fract(g - 0.5) - 0.5) / max(w, vec2<f32>(1e-12));
    let on = 1.0 - min(min(off.x, off.y), 1.0);
    return on * (1.0 - smoothstep(0.125, 0.25, max(w.x, w.y)));
}

// The axis colors, display-linear: x red, y green, z blue.
fn axis_color(a: u32) -> vec3<f32> {
    switch a {
        case 0u: { return vec3<f32>(0.55, 0.08, 0.08); }
        case 1u: { return vec3<f32>(0.1, 0.42, 0.06); }
        default: { return vec3<f32>(0.08, 0.16, 0.6); }
    }
}

// Where a pixel's ray meets one plane of the floor grid: how far along the ray (0: it does not,
// or the plane is off), its lines' display-linear color with their coverage in alpha, and
// whether it lies in front of whatever is drawn there.
struct Plane {
    t: f32,
    lines: vec4<f32>,
    front: bool,
};

const NO_PLANE: Plane = Plane(0.0, vec4<f32>(0.0), false);

// Plane `a` of the floor grid (canvas.floor-grid, research 14 section 3.3), square to axis a, on
// the ray from `start` along `dir` (world axes, from the eye; `start_v` and `dir_v` the same ray
// in view space), at frame position `pixel` over reversed depth `d`. Worked out from the eye
// (grid.rs `Floor`), so a close view of the floor stays exact.
fn floor_plane(a: u32, start: vec3<f32>, dir: vec3<f32>, start_v: vec3<f32>, dir_v: vec3<f32>, pixel: vec2<f32>, d: f32) -> Plane {
    let u = (a + 1u) % 3u;
    let v = (a + 2u) % 3u;
    let t = (f.grid_plane[a] - start[a]) / select(dir[a], 1e-30, dir[a] == 0.0);
    let ahead = t > 0.0 && dir[a] != 0.0;
    let along = start + dir * select(0.0, t, ahead);
    let at = vec2<f32>(along[u], along[v]);
    // Derivatives first, while every pixel of the quad takes this path.
    let fine = (vec2<f32>(f.grid_past[u], f.grid_past[v]) + at) / f.grid.x;
    let coarse = fine * f.grid.z;
    let world = vec2<f32>(f.grid_eye[u], f.grid_eye[v]) + at;
    let w_fine = fwidthFine(fine);
    let w_coarse = fwidthFine(coarse);
    let w_axes = fwidthFine(world);
    let grid = max(lines(fine, w_fine) * f.grid.y, lines(coarse, w_coarse));
    // The axis line along u lies where v is 0, and the other way round.
    let on_u = 1.0 - min(abs(world.y) / max(w_axes.y, 1e-30), 1.0);
    let on_v = 1.0 - min(abs(world.x) / max(w_axes.x, 1e-30), 1.0);
    let off = abs(at - vec2<f32>(f.grid_center[u], f.grid_center[v]));
    let inside = f.grid.w == 0.0 || max(off.x, off.y) <= f.grid.w;
    let clip = f.proj * vec4<f32>(start_v + dir_v * select(0.0, t, ahead), 1.0);
    let depth = clip.z / clip.w;
    let hit = ahead && inside && depth >= 0.0 && in_viewport(vec2<i32>(floor(pixel)));
    // Fade toward the horizon, where the plane is seen edge-on.
    let facing = clamp(abs(dir[a]) * 8.0, 0.0, 1.0);
    var c = vec4<f32>(mix(vec3<f32>(0.32), axis_color(a), f.grid_past.w), 0.5 * grid);
    // The lower axis's line first, so where they cross the higher one is on top.
    let swap = v < u;
    c = mix(c, vec4<f32>(axis_color(min(u, v)), 0.9), select(on_u, on_v, swap));
    c = mix(c, vec4<f32>(axis_color(max(u, v)), 0.9), select(on_v, on_u, swap));
    return Plane(select(0.0, t, hit), vec4<f32>(c.rgb, c.a * facing), depth >= d);
}

// The floor grid's planes at frame position `pixel` over reversed depth `d`, farthest first.
fn floor_planes(pixel: vec2<f32>, d: f32) -> array<Plane, 3> {
    let near = view_point(pixel, view_z(1.0));
    let ortho = f.shading.w == 1u;
    // The ray in view space: from the eye (perspective) or from the eye's plane (orthographic).
    let dir_v = select(normalize(near), vec3<f32>(0.0, 0.0, -1.0), ortho);
    let start_v = select(vec3<f32>(0.0), vec3<f32>(near.xy, 0.0), ortho);
    let to_world = transpose(mat3x3<f32>(f.view[0].xyz, f.view[1].xyz, f.view[2].xyz));
    let dir = to_world * dir_v;
    let start = to_world * start_v;
    var p = array<Plane, 3>(NO_PLANE, NO_PLANE, NO_PLANE);
    for (var a = 0u; a < 3u; a++) {
        if (f.flags.z & (1u << a)) != 0u {
            p[a] = floor_plane(a, start, dir, start_v, dir_v, pixel, d);
        }
    }
    // Compare-and-swaps of 0 and 1, 1 and 2, then 0 and 1 put the farthest first.
    for (var i = 0u; i < 3u; i++) {
        let k = select(0u, 1u, i == 1u);
        let j = k + 1u;
        if p[j].t > p[k].t {
            let swap = p[j];
            p[j] = p[k];
            p[k] = swap;
        }
    }
    return p;
}

// The background at frame position `pixel` (catalog item `canvas.background`), display-linear:
// its color, with a black-to-white gradient down the viewport over it by `f.gradient.x`, mixed
// in sRGB as the color is given. The gradient is half white at its center and runs from white
// to black over its softness (at least one pixel) of the viewport's height.
fn backdrop(pixel: vec2<f32>) -> vec3<f32> {
    let g = f.gradient;
    if g.x <= 0.0 {
        return f.background.rgb;
    }
    let down = (pixel.y - f.viewport.y) / f.viewport.w;
    let white = clamp(0.5 - (down - g.y) / max(g.z, 1.0 / f.viewport.w), 0.0, 1.0);
    return to_linear(mix(to_srgb(f.background.rgb), vec3<f32>(white), g.x));
}

// `c` with floor plane `p` over it. Behind every surface a plane is a backdrop: its fill (the
// background `back`) hides the planes behind it by the fill mode, then its lines. In front of
// a surface only its lines are drawn, so the floor never hides a part.
fn over_plane(c: vec3<f32>, p: Plane, back: vec3<f32>) -> vec3<f32> {
    if p.t <= 0.0 {
        return c;
    }
    let hide = f.grid_plane.w;
    let filled = select(select(mix(c, back, hide), back, hide >= 1.0), c, p.front);
    return mix(filled, p.lines.rgb, p.lines.a);
}

// How opaque the surface at frame position `pixel` over reversed depth `d` is drawn: in fill
// mode 3, Schlick's Fresnel term from a quarter where it faces the eye to whole edge-on, so the
// grid and background show through; otherwise whole.
fn surface_opacity(pixel: vec2<f32>, at: vec2<i32>, d: f32) -> f32 {
    if f.grid_eye.w != 1.0 {
        return 1.0;
    }
    let n = oct_decode(textureLoad(surface, at, 0).xy);
    let eye = select(-normalize(view_point(pixel, view_z(d))), vec3<f32>(0.0, 0.0, 1.0), f.shading.w == 1u);
    let facing = clamp(dot(n, eye), 0.0, 1.0);
    return 0.25 + 0.75 * pow(1.0 - facing, 5.0);
}

struct Out {
    // The frame's color, sRGB-encoded (written through the frame's non-sRGB view).
    @location(0) color: vec4<f32>,
    // The running mean of display-linear color, and of dither coverage in alpha.
    @location(1) history: vec4<f32>,
};

// The 8-bit sRGB color of a mean, with triangular noise from -1 to +1 levels on what surfaces
// cover (the mean's alpha).
fn encode(at: vec2<i32>, mean: vec4<f32>) -> vec4<f32> {
    var e = to_srgb(clamp(mean.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    let q = vec2<u32>(at);
    e += (hash(q, 1u) + hash(q, 2u) - 1.0) / 255.0 * mean.a;
    return vec4<f32>(clamp(e, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

// v rounded to half floats, as the history stores it: a frame then shows exactly what the
// history holds, so a settled frame (fs_show) repeats the last sample's frame byte for byte.
fn as_stored(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(unpack2x16float(pack2x16float(v.xy)), unpack2x16float(pack2x16float(v.zw)));
}

// A new sample: shown over the background and the floor planes behind it, the planes in front
// over it, averaged into the history (sample 0 starts it).
@fragment fn fs(@builtin(position) pos: vec4<f32>) -> Out {
    let at = vec2<i32>(floor(pos.xy));
    let s = light(pos.xy);
    let d = textureLoad(depth, at, 0);
    var planes = array<Plane, 3>(NO_PLANE, NO_PLANE, NO_PLANE);
    if f.flags.z != 0u {
        planes = floor_planes(pos.xy, d);
    }
    let back = backdrop(pos.xy);
    var shown = back;
    var dither = 0.0;
    let physical = s.a == f32(KIND_PHYSICAL);
    let image = s.a == f32(KIND_IMAGE);
    if physical {
        shown = view_transform(s.rgb * f.display.x);
    } else if image {
        shown = clamp(s.rgb * f.display.x, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if s.a > 0.0 {
        shown = clamp(s.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    // The wire's edges (catalog item `canvas.wireframe-view`): the surface target's coverage is
    // 0 on an edge and 1 away from every edge.
    if s.a > 0.0 && f.display.w > 0.5 {
        let edge = 1.0 - textureLoad(surface, at, 0).z;
        shown = mix(shown, f.wire.rgb, f.wire.a * edge);
    }
    if (physical || image) && f.flags.y == 1u {
        dither = 1.0;
    }
    // What a see-through surface shows behind it: the background and the planes behind.
    var c = back;
    for (var i = 0u; i < 3u; i++) {
        planes[i].front = planes[i].front && s.a > 0.0;
        if !planes[i].front {
            c = over_plane(c, planes[i], back);
        }
    }
    if s.a > 0.0 {
        let o = surface_opacity(pos.xy, at, d);
        c = select(mix(c, shown, o), shown, o >= 1.0);
    }
    for (var i = 0u; i < 3u; i++) {
        if planes[i].front {
            c = over_plane(c, planes[i], back);
        }
    }
    var mean = vec4<f32>(c, dither);
    if f.flags.w == 1u && in_viewport(at) {
        mean = vec4<f32>(mix(mean.rgb, OUTLINE, outline(at)), mean.a);
    }
    if f.counts.z > 0u {
        mean = mix(textureLoad(history, at, 0), mean, f.display.y);
    }
    mean = as_stored(mean);
    return Out(encode(at, mean), mean);
}

// Every sample is in: the frame shows the history as it is.
@fragment fn fs_show(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(floor(pos.xy));
    return encode(at, textureLoad(history, at, 0));
}
