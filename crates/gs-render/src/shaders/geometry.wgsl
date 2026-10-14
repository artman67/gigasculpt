// The geometry pass (decision 0019): the one pass that draws triangles. It writes reversed
// depth; the surface target: the view-space normal (octahedral), wireframe coverage (1 where
// no wire line is) and the mask (decision 0015); the part flags (decision 0033); and the paint
// (decision 0039, section 9). No light is worked out here; the lighting stage shades from these.

// The part being drawn: its place (decision 0029), part to scene, a move, turn and positive
// scale, so it maps normals as directions too (the scale goes when they are normalized) and
// keeps the winding; and its flags (decision 0033, section 5): bit 0 marks the active part,
// bits 1 and 2 say what its paint comes from (decision 0039, section 9): 0 its material alone,
// 1 its point colors, 2 its face group colors, 3 its polygons' size on screen (the density view).
struct Part {
    model: mat4x4<f32>,
    flags: vec4<u32>,
};

@group(2) @binding(0) var<uniform> part: Part;

const FROM_POINTS: u32 = 1u;
const FROM_GROUPS: u32 = 2u;
const FROM_DENSITY: u32 = 3u;

// The density view's paint (catalog item `canvas.wireframe-view`) at barycentric coordinate
// `b`, by the polygon's size on screen: blue at 2 pixels or less, green at 8, red at 32 or
// more, and between them by its logarithm. The size is 2 over the steepest coordinate's
// change per pixel, the side of a square quad seen square on.
fn density_paint(b: vec3<f32>) -> vec3<f32> {
    let w = fwidth(b);
    let pixels = 2.0 / max(max(w.x, w.y), max(w.z, 1e-6));
    let t = clamp((log2(pixels) - 1.0) / 4.0, 0.0, 1.0);
    let green = vec3<f32>(0.0, 1.0, 0.0);
    let small = mix(vec3<f32>(0.0, 0.0, 1.0), green, min(t * 2.0, 1.0));
    let large = mix(green, vec3<f32>(1.0, 0.0, 0.0), max(t * 2.0 - 1.0, 0.0));
    return select(large, small, t < 0.5);
}

fn source() -> u32 {
    return (part.flags.x >> 1u) & 3u;
}

// Live Boolean's tint (live.rs): bit 3 of the flags, its color's sRGB bytes in `flags.y`.
fn tinted() -> bool {
    return (part.flags.x & 8u) != 0u;
}

// A tinted mesh's clip position 1 part in 65,536 nearer in reversed depth, so it wins over the
// copy of the same faces under it.
fn nudged(clip: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(clip.xy, clip.z * select(1.0, 1.0 + 1.0 / 65536.0, tinted()), clip.w);
}

fn placed(p: vec3<f32>) -> vec4<f32> {
    return part.model * vec4<f32>(p, 1.0);
}

fn placed_normal(n: vec3<f32>) -> vec3<f32> {
    return (part.model * vec4<f32>(n, 0.0)).xyz;
}

// Hidden-edge masks: 4 bits per triangle, 8 per word. Bit k hides the edge opposite corner k.
@group(1) @binding(2) var<storage, read> hidden_edges: array<u32>;

fn hidden(triangle: u32) -> u32 {
    return (hidden_edges[triangle / 8u] >> ((triangle % 8u) * 4u)) & 7u;
}

// The mask (decision 0015): one byte per vertex slot, 0 free to 255 fully masked, 4 per word.
// A mesh without a mask binds a few zero bytes, which every slot then reads.
@group(1) @binding(3) var<storage, read> mask_bytes: array<u32>;

fn mask_at(slot: u32) -> f32 {
    let word = mask_bytes[min(slot / 4u, arrayLength(&mask_bytes) - 1u)];
    return f32((word >> ((slot % 4u) * 8u)) & 255u) / 255.0;
}

// Point colors (decision 0039, section 9): one `Rgba8` word per vertex slot, sRGB bytes with
// red in the low byte. Face group colors: one word per triangle of the index pool. A mesh
// without either binds a few white words, which every slot or triangle then reads.
@group(1) @binding(4) var<storage, read> point_colors: array<u32>;
@group(1) @binding(5) var<storage, read> group_colors: array<u32>;

// An `Rgba8` word's color in linear light, by the sRGB curve. The rasterizer interpolates it
// in linear light; the paint target stores it as sRGB again.
fn linear(word: u32) -> vec3<f32> {
    let c = unpack4x8unorm(word).rgb;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, c / 12.92, c <= vec3<f32>(0.04045));
}

// The paint at vertex slot `slot`: its point color while the part shows them, else white.
fn point_paint(slot: u32) -> vec3<f32> {
    if source() != FROM_POINTS {
        return vec3<f32>(1.0);
    }
    return linear(point_colors[min(slot, arrayLength(&point_colors) - 1u)]);
}

// The color of the face group of triangle `triangle` of the index pool.
fn group_paint(triangle: u32) -> vec3<f32> {
    return linear(group_colors[min(triangle, arrayLength(&group_colors) - 1u)]);
}

// What a pixel of the geometry pass writes: the surface target, the part's flags and the paint.
struct Written {
    @location(0) surface: vec4<f32>,
    @location(1) parts: u32,
    @location(2) paint: vec4<f32>,
};

// The surface target's value for world normal `n_in` (turned toward the viewer on back faces,
// so open meshes shade from the side seen) with wireframe coverage `wire` and the mask, the
// part's flags, and `paint`.
fn surface(n_in: vec3<f32>, front: bool, wire: f32, mask: f32, paint: vec3<f32>) -> Written {
    let n_world = normalize(select(-n_in, n_in, front));
    let n = normalize((f.view * vec4<f32>(n_world, 0.0)).xyz);
    let tint = select(vec3<f32>(1.0), linear(part.flags.y), tinted());
    return Written(vec4<f32>(oct_encode(n), wire, mask), part.flags.x, vec4<f32>(paint * tint, 1.0));
}

// How much of the surface shows past the wire lines: 0 on a line about 1.2 pixels wide where a
// barycentric coordinate is near 0, 1 away from it. A hidden edge gets 1 added to its
// coordinate, so it never comes near 0. All 1 while the view draws no wire (a part showing
// face groups takes a wire path then).
fn wire_coverage(b_in: vec3<f32>, hide: u32) -> f32 {
    let b = b_in + vec3<f32>(vec3<u32>(hide, hide >> 1u, hide >> 2u) & vec3<u32>(1u));
    let w = max(fwidth(b) * 1.2, vec3<f32>(1e-6));
    let a = smoothstep(vec3<f32>(0.0), w, b);
    return select(1.0, min(min(a.x, a.y), a.z), f.display.w > 0.5);
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) n_world: vec3<f32>,
    // The chunk's first triangle, passed as first_instance by the indirect draw.
    @location(1) @interpolate(flat) first_triangle: u32,
    @location(2) mask: f32,
    @location(3) paint: vec3<f32>,
};

// Indexed draws use base vertex 0, so vertex_index is the vertex's slot in the pool.
@vertex fn vs(@location(0) p: vec3<f32>, @location(1) e: vec2<f32>, @builtin(instance_index) first_triangle: u32, @builtin(vertex_index) slot: u32) -> VsOut {
    let m = deformed(p, oct_decode(e), slot);
    return VsOut(nudged(f.proj * (f.view * placed(m.p))), placed_normal(m.n), first_triangle, mask_at(slot), point_paint(slot));
}

@fragment fn fs(i: VsOut, @builtin(front_facing) front: bool) -> Written {
    return surface(i.n_world, front, 1.0, i.mask, i.paint);
}

// Wireframe fallback (docs/spikes/golden.md): draw 3 vertices per triangle with no index
// buffer; the vertex shader reads both pools itself and makes its own barycentric coordinate
// from vertex_index % 3. vertex_index counts from the chunk's first index (first_vertex).
@group(1) @binding(0) var<storage, read> pool_vertices: array<vec4<u32>>;
@group(1) @binding(1) var<storage, read> pool_indices: array<u32>;

struct PullOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) n_world: vec3<f32>,
    @location(1) @interpolate(flat) hide: u32,
    @location(2) bary: vec3<f32>,
    @location(3) mask: f32,
    @location(4) paint: vec3<f32>,
};

@vertex fn vs_pull(@builtin(vertex_index) vi: u32) -> PullOut {
    let slot = pool_indices[vi];
    let raw = pool_vertices[slot];
    let m = deformed(bitcast<vec3<f32>>(raw.xyz), oct_decode(unpack2x16snorm(raw.w)), slot);
    let clip = nudged(f.proj * (f.view * placed(m.p)));
    let corner = vi % 3u;
    let bary = vec3<f32>(f32(corner == 0u), f32(corner == 1u), f32(corner == 2u));
    let triangle = vi / 3u;
    let paint = select(point_paint(slot), group_paint(triangle), source() == FROM_GROUPS);
    return PullOut(clip, placed_normal(m.n), hidden(triangle), bary, mask_at(slot), paint);
}

@fragment fn fs_pull(i: PullOut, @builtin(front_facing) front: bool) -> Written {
    let paint = select(i.paint, density_paint(i.bary), source() == FROM_DENSITY);
    return surface(i.n_world, front, wire_coverage(i.bary, i.hide), i.mask, paint);
}
