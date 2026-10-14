// Ambient occlusion (decision 0019): GTAO (Jimenez and colleagues, "Practical Real-Time
// Strategies for Accurate Indirect Occlusion", SIGGRAPH 2016) in the form of Intel's XeGTAO
// (MIT, Copyright (C) 2016-2021 Intel Corporation), written for this frame: positions from
// reversed depth, normals from the surface target, a radius in scene units, and a depth-aware
// 5 by 5 filter in its own pass. Both passes run at the occlusion targets' resolution: one
// texel per frame pixel, or one per 2 by 2 pixels on the integrated GPU, where the lighting
// pass scales it back up guided by depth. Each texel holds visibility and the view z of the
// frame pixel it stands for (NOTHING where nothing was drawn).

@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var surface: texture_2d<f32>;
// The occlusion pass's output, read by the filter.
@group(0) @binding(3) var raw: texture_2d<f32>;

// XeGTAO's defaults: the outer 61.5% of the radius fades out; steps are spread by a square
// (closer steps are denser); the first step is 1.3 pixels out, so a pixel never occludes itself.
const FALLOFF_RANGE: f32 = 0.615;
const DISTRIBUTION_POWER: f32 = 2.0;
const TOO_CLOSE: f32 = 1.3;
// The view z of an occlusion texel where nothing was drawn (nothing in view is behind the eye).
const NOTHING: f32 = 1e30;

// Frame pixels per occlusion texel along each side.
fn scale() -> i32 {
    return i32(f.counts.w);
}

fn normal_at(q: vec2<i32>) -> vec3<f32> {
    return oct_decode(textureLoad(surface, q, 0).xy);
}

// XeGTAO's FastACos: within about 0.007 radians of acos, at a fraction of its cost.
fn fast_acos(x: f32) -> f32 {
    let a = abs(x);
    let r = (-0.156583 * a + HALF_PI) * sqrt(1.0 - a);
    return select(PI - r, r, x >= 0.0);
}

// The slice angle and step offset of occlusion texel h: 25 consecutive points of the R2
// sequence over a 5 by 5 tile, so every 5 by 5 window holds all 25; each accumulated sample
// takes the next 25.
fn noise(h: vec2<i32>) -> vec2<f32> {
    let tile = u32(h.x % 5 + 5 * (h.y % 5));
    let i = f32(tile + 25u * f.counts.z);
    return fract(vec2<f32>(0.5) + i * vec2<f32>(0.7548776662, 0.5698402910));
}

// How high the horizon seen from p toward the frame pixel at `q` is, as a cosine from the view
// direction v, faded toward `low` (the tangent plane) with the sample's distance.
fn horizon(p: vec3<f32>, v: vec3<f32>, q: vec2<f32>, low: f32, falloff: vec2<f32>) -> f32 {
    let at = vec2<i32>(floor(q));
    if !in_viewport(at) {
        return low;
    }
    let d = textureLoad(depth, at, 0);
    if d <= 0.0 {
        return low;
    }
    let delta = view_point(vec2<f32>(at) + 0.5, view_z(d)) - p;
    let dist = length(delta);
    let weight = clamp(dist * falloff.x + falloff.y, 0.0, 1.0);
    return mix(low, dot(delta / max(dist, 1e-30), v), weight);
}

// Visibility of the cosine-weighted hemisphere above the frame pixel occlusion texel h stands
// for, and that pixel's view z.
fn visibility(h: vec2<i32>) -> vec2<f32> {
    let at = h * scale();
    let d = textureLoad(depth, at, 0);
    let radius = f.occlusion.x;
    if d <= 0.0 || radius <= 0.0 || !in_viewport(at) {
        return vec2<f32>(1.0, NOTHING);
    }
    let pixel = vec2<f32>(at) + 0.5;
    let z = view_z(d);
    let p = view_point(pixel, z);
    let n = normal_at(at);
    let v = toward_eye(p);
    let reach = radius / pixel_size(p);
    if reach < 1.0 {
        return vec2<f32>(1.0, z);
    }
    let range = FALLOFF_RANGE * radius;
    let falloff = vec2<f32>(-1.0 / range, radius * (1.0 - FALLOFF_RANGE) / range + 1.0);
    let xi = noise(h);
    let slices = f.counts.x;
    let steps = f.counts.y;
    var total = 0.0;
    for (var s = 0u; s < slices; s++) {
        let phi = PI * (f32(s) + xi.x) / f32(slices);
        let c = cos(phi);
        let sn = sin(phi);
        // The slice: the screen direction (frame y grows down) and the view direction.
        let screen = vec2<f32>(c, -sn);
        let dir = vec3<f32>(c, sn, 0.0);
        let ortho = dir - dot(dir, v) * v;
        let axis = normalize(cross(ortho, v));
        let pn = n - axis * dot(n, axis);
        let pn_len = length(pn);
        let cos_n = clamp(dot(pn, v) / max(pn_len, 1e-30), 0.0, 1.0);
        let side = select(-1.0, 1.0, dot(ortho, pn) >= 0.0);
        let n_angle = side * fast_acos(cos_n);
        let sin_n = side * sqrt(1.0 - cos_n * cos_n);
        // The tangent plane: cos(n + pi/2) on the positive side, cos(n - pi/2) on the other.
        let low0 = -sin_n;
        let low1 = sin_n;
        var h0 = low0;
        var h1 = low1;
        for (var t = 0u; t < steps; t++) {
            let step_noise = fract(xi.y + f32(s + t * slices) * 0.6180339887);
            var u = (f32(t) + step_noise) / f32(steps);
            u = pow(u, DISTRIBUTION_POWER) + TOO_CLOSE / reach;
            let offset = round(screen * u * reach);
            h0 = max(h0, horizon(p, v, pixel + offset, low0, falloff));
            h1 = max(h1, horizon(p, v, pixel - offset, low1, falloff));
        }
        // The horizon angles from v, on each side, and the closed-form integral between them.
        let a1 = fast_acos(clamp(h0, -1.0, 1.0));
        let a0 = -fast_acos(clamp(h1, -1.0, 1.0));
        let arc0 = (cos_n + 2.0 * a0 * sin_n - cos(2.0 * a0 - n_angle)) / 4.0;
        let arc1 = (cos_n + 2.0 * a1 * sin_n - cos(2.0 * a1 - n_angle)) / 4.0;
        total += pn_len * (arc0 + arc1);
    }
    return vec2<f32>(clamp(total / f32(slices), 0.0, 1.0), z);
}

@fragment fn fs_occlusion(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(visibility(vec2<i32>(floor(pos.xy))), 0.0, 0.0);
}

// The filter: each texel's 5 by 5 window, weighting a neighbor by how near its depth is to
// where the texel's tangent plane puts it (fading out at two texel sizes), so occlusion never
// crosses a silhouette, a jump in depth or a sharp crease.
@fragment fn fs_filter(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let h = vec2<i32>(floor(pos.xy));
    let c = textureLoad(raw, h, 0);
    if c.y >= NOTHING {
        return vec4<f32>(1.0, NOTHING, 0.0, 0.0);
    }
    let at = h * scale();
    let n = normal_at(at);
    let texel = pixel_size(view_point(vec2<f32>(at) + 0.5, c.y)) * f32(scale());
    // How view z changes per texel across the tangent plane (view y is up, texels go down).
    let slope = vec2<f32>(-n.x, n.y) / max(n.z, 0.1) * texel;
    let tolerance = 2.0 * texel;
    let size = vec2<i32>(textureDimensions(raw));
    var sum = 0.0;
    var weights = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let o = vec2<i32>(x, y);
            let t = textureLoad(raw, clamp(h + o, vec2<i32>(0), size - 1), 0);
            let expected = c.y + dot(slope, vec2<f32>(o));
            let w = clamp(1.0 - abs(t.y - expected) / tolerance, 0.0, 1.0);
            sum += w * t.x;
            weights += w;
        }
    }
    return vec4<f32>(sum / max(weights, 1e-6), c.y, 0.0, 0.0);
}
