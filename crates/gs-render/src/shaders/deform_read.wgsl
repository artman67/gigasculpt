// Reads back the open deformer's preview (decision 0041, amended 2026-10-12 for DF9a): every
// vertex slot of the pool moved by `deformed`, the vertex stage's own function, its point and
// normal written one after the other, for the test that holds the preview to Accept's points.

@group(1) @binding(0) var<storage, read> read_vertices: array<vec4<u32>>;
@group(1) @binding(1) var<storage, read_write> read_out: array<vec4<f32>>;

@compute @workgroup_size(64)
fn read_preview(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let slot = id.x + id.y * groups.x * 64u;
    if slot >= arrayLength(&read_vertices) {
        return;
    }
    let raw = read_vertices[slot];
    let moved = deformed(bitcast<vec3<f32>>(raw.xyz), oct_decode(unpack2x16snorm(raw.w)), slot);
    read_out[2u * slot] = vec4<f32>(moved.p, 0.0);
    read_out[2u * slot + 1u] = vec4<f32>(moved.n, 0.0);
}
