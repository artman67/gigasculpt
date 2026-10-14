// The lighting stage (decision 0019): each pixel's light from the surface target, the occlusion
// and the view's shading: a MatCap, Lit mode (lit.wgsl; Clay mode is the clay look in it, and
// zebra its stripes over Clay mode, decision 0023), normals, or Flat Color. It runs inside the
// display pass (display.wgsl), which shows what `light` returns: rgb is light (scene-linear for
// physical looks, display colors for display aids), alpha the kind of pixel (0 nothing drawn,
// 1 physical, 2 a display-referred image, 3 a display aid). One pass instead of two saves
// writing and reading a 16-byte-per-pixel light target, about 1.5 ms a frame on the integrated
// GPU at 2560x1440.

@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var surface: texture_2d<f32>;
@group(0) @binding(3) var occlusion: texture_2d<f32>;
// A MatCap's D layer (diffuse response per unit albedo), S layer (the rest), and an imported
// two-sphere MatCap's crease image.
@group(0) @binding(4) var matcap_diffuse: texture_2d<f32>;
@group(0) @binding(5) var matcap_sampler: sampler;
@group(0) @binding(6) var matcap_specular: texture_2d<f32>;
@group(0) @binding(7) var crease_image: texture_2d<f32>;
// Each pixel's paint from the geometry pass (decision 0039, section 9), read in linear light:
// white where nothing is painted.
@group(0) @binding(16) var paint_target: texture_2d<f32>;

const MODE_MATCAP: u32 = 1u;
const MODE_ZEBRA: u32 = 2u;
const MODE_NORMALS: u32 = 3u;
const MODE_FLAT: u32 = 5u;
// Kinds of pixel (the light's alpha): physical light, a display-referred image.
const KIND_PHYSICAL: u32 = 1u;
const KIND_IMAGE: u32 = 2u;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// Zebra's stripes (reflection lines): 10 across the reflected view direction, anti-aliased,
// as a factor on Clay mode's light (0.3 in the dark stripes). Small changes in the surface
// bend the stripes (docs/spikes/golden.md). The stripes' width comes from fine derivatives,
// one per pixel: in a full-screen pass, coarse ones (one per 2 by 2 block, which some drivers
// use for fwidth) made the Radeon's stripe edges differ from lavapipe's by up to 0.23 FLIP;
// fine ones keep it near 0.05.
fn stripes(n: vec3<f32>, v: vec3<f32>) -> f32 {
    let r = reflect(-v, n);
    let s = r.y * 10.0;
    let stripe_at = abs(fract(s) - 0.5) * 2.0;
    let aa = max(fwidthFine(s) * 2.0, 1e-4);
    return 0.3 + 0.7 * smoothstep(0.5 - aa, 0.5 + aa, stripe_at);
}

// Where a MatCap is read for view-space normal n seen along view direction i (toward the
// viewer): n in a frame that turns +z onto i, scaled by 0.496 about the center (Blender's
// Workbench `matcap_uv_compute`, GPL-2.0-or-later). A surface facing the viewer reads the
// image center wherever it is on screen; in orthographic views i is +z and this is n.xy. The
// flip mirrors u, as Blender's MatCap flip does.
fn matcap_uv(i: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    let a = 1.0 / (1.0 + i.z);
    let b = -i.x * i.y * a;
    let b1 = vec3<f32>(1.0 - i.x * i.x * a, b, -i.x);
    let b2 = vec3<f32>(b, 1.0 - i.y * i.y * a, -i.y);
    let uv = vec2<f32>(dot(b1, n), dot(b2, n)) * 0.496;
    return vec2<f32>(0.5 + f.matcap.x * uv.x, 0.5 - uv.y);
}

// GTAO's multi-bounce fit (Jimenez and colleagues 2016, appendix): occlusion that accounts for
// light bouncing between nearby surfaces of albedo r, never below the raw visibility.
fn multi_bounce(a: f32, r: vec3<f32>) -> vec3<f32> {
    let x = 2.0404 * r - 0.3324;
    let y = 0.6417 - 4.7951 * r;
    let z = 2.7552 * r + 0.6903;
    return max(vec3<f32>(a), ((x * a + y) * a + z) * a);
}

// Specular occlusion from visibility a (Lagarde and de Rousiers, "Moving Frostbite to PBR",
// 2014; as in Filament): how much of the reflection cone of GGX alpha `alpha` the occlusion
// blocks, seen at n.v = `nv`.
fn specular_occlusion(nv: f32, a: f32, alpha: f32) -> f32 {
    return clamp(pow(nv + a, exp2(-16.0 * alpha - 1.0)) - 1.0 + a, 0.0, 1.0);
}

// Colorize (decision 0043, step (c)): an occlusion factor o with the material's share of the
// light it took given back in the cavity color. Where nothing is occluded (o = 1) it changes
// nothing, and so does Colorize 0.
fn colorized(o: vec3<f32>) -> vec3<f32> {
    return o + f.cavity.w * f.cavity.rgb * (1.0 - o);
}

// The paint at frame pixel `at`, which multiplies the albedo (decision 0039, section 9); zebra
// and normals show none. A frame where no part shows paint binds one white texel instead.
fn paint_at(at: vec2<i32>) -> vec3<f32> {
    if f.shading.x == MODE_ZEBRA || f.shading.x == MODE_NORMALS {
        return vec3<f32>(1.0);
    }
    let hi = vec2<i32>(textureDimensions(paint_target)) - 1;
    return textureLoad(paint_target, min(at, hi), 0).rgb;
}

// MatCap (material.matcap-shading) with occlusion on the environment light only
// (material.cavity-shading): a baked look's albedo x D darkened by multi-bounce occlusion and S
// by specular occlusion; an imported image darkened by multi-bounce occlusion with its own
// color standing in for the albedo; an imported flat-plus-crease pair blended toward the crease
// image by the occlusion. The paint multiplies the albedo, or the image's color, and S by the
// look's metalness: a metal's S holds its base color and its D is empty (research 11, R7 in
// research 27), so paint shows on metals too. Unpainted or dielectric, S is untouched.
fn matcap(n: vec3<f32>, v: vec3<f32>, a: f32, paint: vec3<f32>) -> vec3<f32> {
    let uv = matcap_uv(v, n);
    let d = textureSampleLevel(matcap_diffuse, matcap_sampler, uv, 0.0).rgb;
    if f.shading.z == 1u {
        let crease = textureSampleLevel(crease_image, matcap_sampler, uv, 0.0).rgb;
        return mix(d, crease, clamp(1.0 - a + f.matcap.y, 0.0, 1.0)) * paint;
    }
    if f.shading.y == KIND_IMAGE {
        let c = d * paint;
        return c * multi_bounce(a, clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    }
    let tint = 1.0 + f.albedo.w * (paint - 1.0);
    let s = textureSampleLevel(matcap_specular, matcap_sampler, uv, 0.0).rgb * tint;
    let albedo = f.albedo.rgb * paint;
    let so = colorized(vec3<f32>(specular_occlusion(max(dot(n, v), 0.0), a, f.occlusion.z)));
    return albedo * d * colorized(multi_bounce(a, albedo)) + s * so;
}

// Visibility at the frame pixel centered at `pixel`, view-space point p with normal n, from the
// occlusion texels around it: bilinear weights, each scaled down the farther the texel's
// depth is from p's (relative to the size of a texel there, more on surfaces seen edge-on),
// so the lower resolution of the integrated GPU never pulls occlusion across an edge. At one
// texel per pixel this reads that pixel's texel.
fn occlusion_at(pixel: vec2<f32>, p: vec3<f32>, n: vec3<f32>) -> f32 {
    let s = f32(f.counts.w);
    let u = (pixel - 0.5) / s;
    let base = vec2<i32>(floor(u));
    let fr = u - floor(u);
    let tolerance = 2.0 * pixel_size(p) * s / max(n.z, 0.1);
    let size = vec2<i32>(textureDimensions(occlusion));
    var sum = 0.0;
    var weights = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1u);
        let t = textureLoad(occlusion, clamp(base + o, vec2<i32>(0), size - 1), 0);
        let bilinear = select(1.0 - fr.x, fr.x, o.x == 1) * select(1.0 - fr.y, fr.y, o.y == 1);
        let dz = (t.y - p.z) / tolerance;
        let w = bilinear / (1.0 + dz * dz);
        sum += w * t.x;
        weights += w;
    }
    return select(1.0, sum / weights, weights > 1e-6);
}

// The visibility occlusion leaves at frame position `pixel` (view point p, normal n): 1 with
// occlusion off.
fn visibility(pixel: vec2<f32>, p: vec3<f32>, n: vec3<f32>) -> f32 {
    if f.occlusion.y > 0.0 {
        return max(1.0 - f.occlusion.y * (1.0 - occlusion_at(pixel, p, n)), 0.0);
    }
    return 1.0;
}

// The light at frame position `pixel`. Call it where every pixel of a 2 by 2 block calls it
// (zebra takes screen derivatives).
fn light(pixel: vec2<f32>) -> vec4<f32> {
    let at = vec2<i32>(floor(pixel));
    let d = textureLoad(depth, at, 0);
    let g = textureLoad(surface, at, 0);
    let n = oct_decode(g.xy);
    // Behind nothing drawn the depth is 0; any point in front of the eye stands in, so zebra's
    // stripes, which take screen derivatives, are worked out for every pixel.
    let p = view_point(pixel, view_z(max(d, 1e-6)));
    let v = toward_eye(p);
    var k = 1.0;
    if f.shading.x == MODE_ZEBRA {
        k = stripes(n, v);
    }
    // Nothing else is worked out where nothing is drawn: most of a fitted view's pixels, and
    // Lit's light per pixel costs about twice a MatCap's on the integrated GPU.
    if d <= 0.0 {
        return vec4<f32>(0.0);
    }
    let a = visibility(pixel, p, n);
    let paint = paint_at(at);
    var c: vec3<f32>;
    switch f.shading.x {
        case MODE_MATCAP: { c = matcap(n, v, a, paint); }
        // World-space normal as color, so the stored byte is n * 127.5 + 127.5.
        case MODE_NORMALS: { c = to_linear((vec4<f32>(n, 0.0) * f.view).xyz * 0.5 + 0.5); }
        // Flat Color: the paint times the base color, unlit (decision 0043, step (c)).
        case MODE_FLAT: { c = f.albedo.rgb * paint; }
        // Clay, zebra and Lit.
        default: {
            var s = shade_lit(n, v, a, paint);
            if lit.scatter.w > 0.0 {
                s.irradiance = scattered(pixel, p, s.irradiance);
            }
            c = (s.weight * s.irradiance + s.specular) * k;
        }
    }
    // Overlays on the surface's own color, in linear light: the mask darkens what it covers
    // by the view's mask opacity (decision 0015; 0 while hidden). The display pass draws the
    // wire lines over the finished picture.
    c *= 1.0 - f.display.z * g.w;
    return vec4<f32>(c, f32(f.shading.y));
}
