// Copies the finished frame onto the window's surface texture, pixel for pixel. The frame is
// read through a view with the same sRGB-ness as the surface, so the bytes arrive unchanged.

@group(0) @binding(0) var frame: texture_2d<f32>;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the screen: (-1, -1), (3, -1), (-1, 3).
    let corner = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(frame, vec2<i32>(p.xy), 0);
}
