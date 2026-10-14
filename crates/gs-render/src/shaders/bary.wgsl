// Native wireframe: hardware barycentrics, and the triangle number from the chunk's first
// triangle plus primitive_index, which also finds the triangle's face group color. Needs
// SHADER_BARYCENTRICS and PRIMITIVE_INDEX.

@fragment fn fs_bary(i: VsOut, @builtin(barycentric) b: vec3<f32>, @builtin(primitive_index) prim: u32, @builtin(front_facing) front: bool) -> Written {
    let triangle = i.first_triangle + prim;
    let groups = select(i.paint, group_paint(triangle), source() == FROM_GROUPS);
    let paint = select(groups, density_paint(b), source() == FROM_DENSITY);
    return surface(i.n_world, front, wire_coverage(b, hidden(triangle)), i.mask, paint);
}
