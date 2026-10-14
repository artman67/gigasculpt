// The interface layer: textured, vertex-colored triangles in physical pixels. Colors and
// textures are sRGB-encoded and premultiplied, and blending happens on those encoded values
// (the render target is a non-sRGB view), which is how the interface toolkit expects it.

struct Screen {
    size: vec2<f32>,
    pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex fn vs(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    let ndc = vec2<f32>(2.0 * pos.x / screen.size.x - 1.0, 1.0 - 2.0 * pos.y / screen.size.y);
    return VsOut(vec4<f32>(ndc, 0.0, 1.0), uv, color);
}

@fragment fn fs(i: VsOut) -> @location(0) vec4<f32> {
    return i.color * textureSample(image, image_sampler, i.uv);
}
