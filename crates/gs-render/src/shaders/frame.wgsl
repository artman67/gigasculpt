// Shared by every pass of the frame (decision 0019): the frame's uniforms (`FrameUniforms` in
// uniforms.rs), normal encoding, positions from reversed depth, and the full-screen triangle.

struct Frame {
    // World to view, and view to clip (jittered by the sample's sub-pixel offset).
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    // The scene's viewport in frame pixels: x, y, width, height.
    viewport: vec4<f32>,
    // The look's albedo, linear (multiplies a MatCap's D layer; Flat Color's base color), and
    // its metalness (how much paint tints the S layer).
    albedo: vec4<f32>,
    // The background, display-linear.
    background: vec4<f32>,
    // Shading mode, kind of pixel written, 1 when the MatCap has a crease image, 1 when
    // orthographic.
    shading: vec4<u32>,
    // Occlusion radius in scene units (0: off), strength (times the material's cavity
    // Intensity, so up to 2), the look's GGX alpha (roughness squared, for specular occlusion),
    // unused.
    occlusion: vec4<f32>,
    // Occlusion slices and steps per side, the accumulation sample, frame pixels per occlusion
    // texel along each side (1, or 2 on the integrated GPU).
    counts: vec4<u32>,
    // Exposure gain, accumulation weight (1 / (sample + 1); 0 holds the history), how much a
    // fully masked surface is darkened (0 while the mask is hidden), 1 when the wire is drawn
    // (a part showing face groups takes a wire path with it off).
    display: vec4<f32>,
    // View transform, dither on, the floor grid's planes (bits 1 x, 2 y, 4 z; 0: off), the
    // active part's outline on (decision 0033).
    flags: vec4<u32>,
    // The floor grid (grid.rs `Floor`): finer spacing, the finer level's strength, finer
    // spacing over coarser, half a sized grid's side (0: endless).
    grid: vec4<f32>,
    // Each plane's place along its axis minus the eye's; how much a plane's fill hides.
    grid_plane: vec4<f32>,
    // The eye past the coarser line before it along each axis; how much lines take their
    // plane's axis color.
    grid_past: vec4<f32>,
    // The eye; 1 when the parts are see-through where they face the eye (fill mode 3).
    grid_eye: vec4<f32>,
    // A sized grid's center minus the eye; unused.
    grid_center: vec4<f32>,
    // Which way the MatCap lookup's u runs: 1, or -1 flipped (catalog item
    // `material.matcap-shading`); Cavity Transition, -1 to 1, which leans a flat-plus-crease
    // pair's blend (`material.cavity-shading`); unused, unused.
    matcap: vec4<f32>,
    // The wire's edges: display-linear color, and how much an edge covers the surface.
    wire: vec4<f32>,
    // The background's gradient (catalog item `canvas.background`): how much of it shows (0:
    // the flat background), where down the viewport it is half white, how much of the
    // viewport's height it spreads over; unused.
    gradient: vec4<f32>,
    // The material's cavity color, linear, and its Colorize (decision 0043, step (c)): 0 in
    // every shading but MatCap and Lit.
    cavity: vec4<f32>,
};

@group(0) @binding(0) var<uniform> f: Frame;

const PI: f32 = 3.14159265358979;
const HALF_PI: f32 = 1.57079632679490;

// Octahedral unit vector <-> two numbers in -1..1.
fn oct_decode(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e.x, e.y, 1.0 - abs(e.x) - abs(e.y));
    let t = max(-n.z, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.y += select(t, -t, n.y >= 0.0);
    return normalize(n);
}

fn oct_encode(n: vec3<f32>) -> vec2<f32> {
    let p = n.xy / (abs(n.x) + abs(n.y) + abs(n.z));
    let flipped = (1.0 - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    return select(p, flipped, n.z < 0.0);
}

// Normalized device coordinates of frame position `pixel` (pixel centers at .5) in the viewport.
fn ndc(pixel: vec2<f32>) -> vec2<f32> {
    let uv = (pixel - f.viewport.xy) / f.viewport.zw;
    return vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
}

// View-space z (negative in front of the eye) for reversed depth `d` above 0: clip z is
// proj[2][2] z + proj[3][2] and clip w is proj[2][3] z + proj[3][3].
fn view_z(d: f32) -> f32 {
    return (f.proj[3][2] - d * f.proj[3][3]) / (d * f.proj[2][3] - f.proj[2][2]);
}

// The view-space point at frame position `pixel` (pixel centers at .5) whose view z is `z`.
fn view_point(pixel: vec2<f32>, z: f32) -> vec3<f32> {
    let xy = ndc(pixel);
    let w = f.proj[2][3] * z + f.proj[3][3];
    let x = (xy.x * w - f.proj[2][0] * z - f.proj[3][0]) / f.proj[0][0];
    let y = (xy.y * w - f.proj[2][1] * z - f.proj[3][1]) / f.proj[1][1];
    return vec3<f32>(x, y, z);
}

// Unit vector from a view-space point toward the viewer.
fn toward_eye(p: vec3<f32>) -> vec3<f32> {
    return select(normalize(-p), vec3<f32>(0.0, 0.0, 1.0), f.shading.w == 1u);
}

// Scene size of one pixel at view-space point p (proj[1][1] is 1 / tan(fov / 2) in
// perspective, 1 / half the view height in orthographic).
fn pixel_size(p: vec3<f32>) -> f32 {
    let w = select(-p.z, 1.0, f.shading.w == 1u);
    return 2.0 * w / (f.proj[1][1] * f.viewport.w);
}

// True when frame pixel `q` lies in the viewport.
fn in_viewport(q: vec2<i32>) -> bool {
    let lo = vec2<i32>(f.viewport.xy);
    let hi = lo + vec2<i32>(f.viewport.zw);
    return all(q >= lo) && all(q < hi);
}

// One triangle covering the target; position only.
@vertex fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}
