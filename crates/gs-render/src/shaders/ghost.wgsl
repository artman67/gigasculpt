// Live Boolean's ghosts (live.rs, decision 0057 section 9 as amended for LB7): a Difference or
// Intersect operand drawn as an outline only, about 2 pixels wide just outside its silhouette,
// seen through everything. The cover pass draws each ghost flat in its color (the part's
// `flags.y`, sRGB bytes) into a target of its own, with no depth; the ring pass then colors
// each pixel the cover leaves bare, within 2 pixels of it, with its nearest covered pixel's
// color, straight into the frame. Comes after geometry.wgsl, whose `vs` and `part` it uses.

@fragment fn fs_ghost(i: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(unpack4x8unorm(part.flags.y).rgb, 1.0);
}

@group(0) @binding(1) var ghost_cover: texture_2d<f32>;

@fragment fn fs_ring(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(at.xy);
    let hi = vec2<i32>(textureDimensions(ghost_cover)) - 1;
    if textureLoad(ghost_cover, p, 0).a > 0.0 {
        discard;
    }
    // The 12 pixels within 2 pixels, nearest first: a quarter turn, the diagonals, two steps.
    var ring = array<vec2<i32>, 12>(
        vec2<i32>(1, 0), vec2<i32>(-1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1),
        vec2<i32>(1, 1), vec2<i32>(-1, 1), vec2<i32>(1, -1), vec2<i32>(-1, -1),
        vec2<i32>(2, 0), vec2<i32>(-2, 0), vec2<i32>(0, 2), vec2<i32>(0, -2),
    );
    for (var k = 0; k < 12; k++) {
        let c = textureLoad(ghost_cover, clamp(p + ring[k], vec2<i32>(0), hi), 0);
        if c.a > 0.0 {
            return vec4<f32>(c.rgb, 1.0);
        }
    }
    discard;
    return vec4<f32>(0.0);
}
