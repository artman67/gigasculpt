// GPU culling per chunk (see cull.rs). One workgroup: each lane tests a contiguous run of
// chunks, the lanes prefix-sum their counts, then each lane writes the draw records of its
// visible chunks at its offset. Visible chunks keep their order, so a culled frame draws what an
// unculled one draws, minus chunks that cannot reach the screen. Every test is written so that a
// NaN fails it and the chunk is drawn.

struct Chunk {
    center: vec3<f32>,
    radius: f32,
    axis: vec3<f32>,
    cos_half: f32,
    sin_half: f32,
    first_index: u32,
    index_count: u32,
    first_triangle: u32,
};

// DrawIndexedIndirectArgs.
struct Draw {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
};

struct CullCamera {
    planes: array<vec4<f32>, 4>,
    eye: vec4<f32>,
    forward: vec4<f32>,
    count: u32,
    cone: u32,
    ortho: u32,
    pad: u32,
};

@group(0) @binding(0) var<uniform> cam: CullCamera;
@group(0) @binding(1) var<storage, read> chunks: array<Chunk>;
@group(0) @binding(2) var<storage, read_write> draws: array<Draw>;
@group(0) @binding(3) var<storage, read_write> drawn: array<u32>;

const LANES: u32 = 256u;
// A chunk is skipped for facing away only if every triangle faces away by more than this
// (about 0.9 degrees), so the rasterizer's own rounding never sees one of them facing us.
const FACING_MARGIN: f32 = 0.015625;

var<workgroup> sums: array<u32, LANES>;

fn visible(c: Chunk) -> bool {
    if (c.index_count == 0u) {
        return false;
    }
    let d = c.center - cam.eye.xyz;
    let dist = length(d);
    let slack = c.radius * 1.001 + 1e-5 * (dist + c.radius);
    for (var i = 0u; i < 4u; i++) {
        let p = cam.planes[i];
        if (dot(p.xyz, c.center) + p.w < -slack) {
            return false;
        }
    }
    if (cam.cone == 0u) {
        return true;
    }
    // The least any normal in the cone points along v: |v| cos(angle(axis, v) + half angle).
    let v = select(d, cam.forward.xyz, cam.ortho == 1u);
    let reach = select(c.radius + dist * FACING_MARGIN, FACING_MARGIN, cam.ortho == 1u);
    let away = dot(c.axis, v) * c.cos_half - length(cross(c.axis, v)) * c.sin_half;
    return !(away > reach);
}

@compute @workgroup_size(256)
fn cull(@builtin(local_invocation_index) lane: u32) {
    let per = (cam.count + LANES - 1u) / LANES;
    let start = min(lane * per, cam.count);
    let end = min(start + per, cam.count);
    var mine = 0u;
    for (var i = start; i < end; i++) {
        if (visible(chunks[i])) {
            mine += 1u;
        }
    }
    sums[lane] = mine;
    workgroupBarrier();
    for (var step = 1u; step < LANES; step <<= 1u) {
        var before = 0u;
        if (lane >= step) {
            before = sums[lane - step];
        }
        workgroupBarrier();
        sums[lane] += before;
        workgroupBarrier();
    }
    var at = sums[lane] - mine;
    for (var i = start; i < end; i++) {
        let c = chunks[i];
        if (visible(c)) {
            draws[at] = Draw(c.index_count, 1u, c.first_index, 0, c.first_triangle);
            at += 1u;
        }
    }
    if (lane == LANES - 1u) {
        drawn[0] = sums[lane];
    }
}
