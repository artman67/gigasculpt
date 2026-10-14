// Lit mode (decision 0023): a material lit live by the environment and the disc lights, inside
// the lighting stage (lighting.wgsl). The same light as `shading::lit::shade` on the CPU, which
// it is checked against: the bake's layering (the DFG table's albedos with Filament's
// compensation, the body's share 1 - E, the coat over the base), the environment's SH9
// irradiance with multi-bounce occlusion, its split-sum reflections with specular occlusion,
// and each light's cap irradiance and representative-point highlight, which occlusion leaves
// alone. Directions are view space: the environment and the lights turn with the camera. Live
// subsurface spreads the diffuse irradiance under the surface (`scattered`, fed by the diffuse
// pass `fs_diffuse`).

struct Lit {
    // The dielectric's, the metal's and the coat's Fresnel curves: f0 with the lobe's
    // perceptual roughness in w, f90, and the F82 dip's b.
    f0: array<vec4<f32>, 3>,
    f90: array<vec4<f32>, 3>,
    dip: array<vec4<f32>, 3>,
    // Base color (linear) and metalness.
    base: vec4<f32>,
    // Coat color and weight.
    coat: vec4<f32>,
    // The environment's SH9 irradiance as polynomial coefficients (`Sh9::polynomial`).
    sh: array<vec4<f32>, 9>,
    // Each light: toward its center with sin of its angular radius in w, and its illuminance.
    toward: array<vec4<f32>, 8>,
    illuminance: array<vec4<f32>, 8>,
    // Lights on, unused, unused, unused.
    counts: vec4<u32>,
    // Ambient (scales the environment's light), unused, unused, unused.
    params: vec4<f32>,
    // Live subsurface: each channel's Burley shape d in scene units, and the share of the
    // diffuse light that scatters (0: subsurface off).
    scatter: vec4<f32>,
    // The gather's radii in units of the farthest-reaching channel's d, nearest first (21).
    radii: array<vec4<f32>, 6>,
};

@group(0) @binding(9) var<uniform> lit: Lit;
// Everything in the environment but light 1, prefiltered for GGX: mip i at roughness
// `level_roughness(i)`.
@group(0) @binding(10) var environment: texture_cube<f32>;
@group(0) @binding(11) var environment_sampler: sampler;
// The DFG table: row j roughness j / 63, column i view cosine i / 63; (total, schlick, dip).
@group(0) @binding(12) var dfg: texture_2d<f32>;
// Each pixel's diffuse irradiance, written by the diffuse pass while subsurface runs.
@group(0) @binding(13) var diffuse_light: texture_2d<f32>;

const DFG_LAST: f32 = 63.0;
const PREFILTER_LAST: f32 = 4.0;
const MIN_ALPHA: f32 = 1e-4;
const SCATTER_SAMPLES: u32 = 21u;
const GOLDEN_ANGLE: f32 = 2.39996323;

fn ggx_alpha(roughness: f32) -> f32 {
    return max(roughness * roughness, MIN_ALPHA);
}

// The DFG table at view cosine `mu` and `roughness`, bilinear as `Dfg::at`.
fn dfg_at(mu: f32, roughness: f32) -> vec3<f32> {
    let x = clamp(mu, 0.0, 1.0) * DFG_LAST;
    let y = clamp(roughness, 0.0, 1.0) * DFG_LAST;
    let i = min(i32(x), 62);
    let j = min(i32(y), 62);
    let fx = x - f32(i);
    let fy = y - f32(j);
    let a = textureLoad(dfg, vec2<i32>(i, j), 0).xyz;
    let b = textureLoad(dfg, vec2<i32>(i + 1, j), 0).xyz;
    let c = textureLoad(dfg, vec2<i32>(i, j + 1), 0).xyz;
    let d = textureLoad(dfg, vec2<i32>(i + 1, j + 1), 0).xyz;
    let lo = a + (b - a) * fx;
    let hi = c + (d - c) * fx;
    return lo + (hi - lo) * fy;
}

// Filament's multiple-scattering compensation for lobe `k` with integrals `e`.
fn compensation(e: vec3<f32>, k: u32) -> vec3<f32> {
    return 1.0 + lit.f0[k].rgb * (1.0 / max(e.x, 1e-4) - 1.0);
}

// Lobe `k`'s compensated directional albedo for the integrals `e` (`Albedo::compensated`).
fn lobe_albedo(e: vec3<f32>, k: u32) -> vec3<f32> {
    let raw = lit.f0[k].rgb * (e.x - e.y) + lit.f90[k].rgb * e.y - lit.dip[k].rgb * e.z;
    return clamp(raw * compensation(e, k), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Lobe `k`'s Fresnel at cosine `c`.
fn fresnel(c: f32, k: u32) -> vec3<f32> {
    let m = clamp(1.0 - c, 0.0, 1.0);
    let m2 = m * m;
    let s = m2 * m2 * m;
    let dip = c * m2 * m2 * m2;
    let f = lit.f0[k].rgb + (lit.f90[k].rgb - lit.f0[k].rgb) * s - lit.dip[k].rgb * dip;
    return clamp(f, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Lobe `k`, `F D V` toward light `l` (brdf.rs `ggx_lobe`).
fn ggx_lobe(k: u32, alpha: f32, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let nv = dot(n, v);
    let nl = dot(n, l);
    if nv <= 0.0 || nl <= 0.0 {
        return vec3<f32>(0.0);
    }
    let sum = v + l;
    let h = select(n, normalize(sum), dot(sum, sum) > 0.0);
    let a2 = alpha * alpha;
    let nh = max(dot(n, h), 0.0);
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let distribution = a2 / (PI * dd * dd);
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    let visibility = 0.5 / (gv + gl);
    return fresnel(max(dot(v, h), 0.0), k) * (distribution * visibility);
}

// The environment's irradiance at unit normal n from its SH9 fit.
fn sh_irradiance(n: vec3<f32>) -> vec3<f32> {
    var e = lit.sh[0].rgb + lit.sh[1].rgb * n.y + lit.sh[2].rgb * n.z + lit.sh[3].rgb * n.x;
    e += lit.sh[4].rgb * (n.x * n.y) + lit.sh[5].rgb * (n.y * n.z);
    e += lit.sh[6].rgb * (3.0 * n.z * n.z - 1.0) + lit.sh[7].rgb * (n.x * n.z);
    e += lit.sh[8].rgb * (n.x * n.x - n.y * n.y);
    return max(e, vec3<f32>(0.0));
}

// Irradiance from a uniform cap of radiance 1 at cosine `cos_t` from the normal, `sin2_s` the
// square of the sine of its angular radius (light.rs `cap_irradiance`).
fn cap_irradiance(cos_t: f32, sin2_s: f32) -> f32 {
    if cos_t * cos_t > sin2_s {
        return PI * sin2_s * max(cos_t, 0.0);
    }
    let sin_t = sqrt(max(1.0 - cos_t * cos_t, 0.0));
    let x = sqrt(max(1.0 / sin2_s - 1.0, 0.0));
    let y = clamp(-x * cos_t / max(sin_t, 1e-12), -1.0, 1.0);
    let root = sin_t * sqrt(max(1.0 - y * y, 0.0));
    let e = (cos_t * acos(y) - x * root) * sin2_s + atan(root / max(x, 1e-12));
    return max(e, 0.0);
}

// The point of light `i`'s disc nearest the ray along r (light.rs `representative`).
fn representative(r: vec3<f32>, i: u32) -> vec3<f32> {
    let l = lit.toward[i].xyz;
    let to_ray = r * dot(l, r) - l;
    let len = length(to_ray);
    let t = select(0.0, min(lit.toward[i].w / len, 1.0), len > 0.0);
    let closest = l + to_ray * t;
    return select(l, normalize(closest), dot(closest, closest) > 0.0);
}

// The prefiltered environment along r, for a lobe of `roughness` around normal n: Filament's
// dominant direction and level (lit.rs `roughness_lod`).
fn reflected(r: vec3<f32>, n: vec3<f32>, roughness: f32) -> vec3<f32> {
    let alpha = ggx_alpha(roughness);
    let dir = normalize(mix(r, n, alpha * alpha));
    let rough = clamp(roughness, 0.0, 1.0);
    let lod = PREFILTER_LAST * rough * (2.0 - rough);
    return textureSampleLevel(environment, environment_sampler, dir, lod).rgb;
}

// The diffuse irradiance at view-space normal n with visibility a on a surface of `albedo`:
// the environment's SH9 fit darkened by multi-bounce occlusion, and each light's cap
// irradiance, which occlusion leaves alone.
fn lit_irradiance(n: vec3<f32>, a: f32, albedo: vec3<f32>) -> vec3<f32> {
    var e = sh_irradiance(n) * lit.params.x * colorized(multi_bounce(a, albedo));
    for (var i = 0u; i < lit.counts.x; i++) {
        let sin_s = lit.toward[i].w;
        let sin2 = sin_s * sin_s;
        e += lit.illuminance[i].rgb * (cap_irradiance(dot(n, lit.toward[i].xyz), sin2) / (PI * sin2));
    }
    return e;
}

// The light leaving a Lit surface, split for live subsurface: the diffuse is weight x
// irradiance (lit.rs `Shaded`).
struct Shaded {
    irradiance: vec3<f32>,
    weight: vec3<f32>,
    specular: vec3<f32>,
};

// The material at view-space normal n seen from v (toward the viewer) with visibility a
// (lit.rs `shade`), its base color multiplied by `paint` (decision 0039, section 9): the
// diffuse's, and the metal's reflections, which carry a metal's base color (research 11, R7 in
// research 27). The dielectric's and the coat's reflections are never tinted.
fn shade_lit(n: vec3<f32>, v: vec3<f32>, a: f32, paint: vec3<f32>) -> Shaded {
    let mu = max(dot(n, v), 1e-4);
    // Variables, not values: they are indexed by the loop below.
    var rough = array<f32, 3>(lit.f0[0].w, lit.f0[1].w, lit.f0[2].w);
    let e0 = dfg_at(mu, rough[0]);
    let e2 = dfg_at(mu, rough[2]);
    let albedo = array<vec3<f32>, 3>(lobe_albedo(e0, 0u), lobe_albedo(e0, 1u), lobe_albedo(e2, 2u));
    // Each lobe's compensation, for the lights (the dielectric and the metal share a roughness).
    var compensated = array<vec3<f32>, 3>(
        compensation(e0, 0u),
        compensation(e0, 1u),
        compensation(e2, 2u),
    );
    let coat_weight = lit.coat.w;
    // Material::leftover: the body's share under the dielectric, the base's under the coat.
    let body = 1.0 - albedo[0];
    let under = (1.0 - coat_weight * albedo[2]) * (1.0 - coat_weight * (1.0 - lit.coat.rgb));
    let metal = lit.base.w;
    var share = array<vec3<f32>, 3>(
        under * (1.0 - metal),
        under * metal * paint,
        vec3<f32>(coat_weight),
    );
    let base = lit.base.rgb * paint;
    var out: Shaded;
    out.weight = share[0] * body * base / PI;
    let r = reflect(-v, n);
    let ambient = lit.params.x;

    out.irradiance = lit_irradiance(n, a, base);
    // The environment's reflections, occluded.
    var alpha = array<f32, 3>(ggx_alpha(rough[0]), ggx_alpha(rough[1]), ggx_alpha(rough[2]));
    let so = colorized(vec3<f32>(specular_occlusion(mu, a, alpha[0])));
    let seen = reflected(r, n, rough[0]) * (so * ambient);
    out.specular = (share[0] * albedo[0] + share[1] * albedo[1]) * seen;
    if coat_weight > 0.0 {
        let coat_so = colorized(vec3<f32>(specular_occlusion(mu, a, alpha[2])));
        let coat_seen = reflected(r, n, rough[2]) * (coat_so * ambient);
        out.specular += share[2] * albedo[2] * coat_seen;
    }

    // The lights' highlights.
    for (var i = 0u; i < lit.counts.x; i++) {
        let sin_s = lit.toward[i].w;
        let e = lit.illuminance[i].rgb;
        let l = representative(r, i);
        let nl = dot(n, l);
        if nl <= 0.0 {
            continue;
        }
        for (var k = 0u; k < 3u; k++) {
            if all(share[k] <= vec3<f32>(0.0)) {
                continue;
            }
            let lobe = ggx_lobe(k, alpha[k], n, v, l) * compensated[k];
            let wider = min(alpha[k] + sin_s / 3.0, 1.0);
            let normalization = (alpha[k] / wider) * (alpha[k] / wider);
            out.specular += share[k] * lobe * (normalization * nl) * e;
        }
    }
    return out;
}

// Burley's normalized diffusion profile at distance r > 0 for each channel's shape d
// (shading/burley.rs).
fn burley(r: f32, d: vec3<f32>) -> vec3<f32> {
    return (exp(-r / d) + exp(-r / (3.0 * d))) / (8.0 * PI * d * r);
}

// Live subsurface (decision 0023; Golubev 2018, Unity HDRP): the diffuse irradiance arriving
// at view point p (frame position `pixel`, whose own irradiance is `own`) after spreading under
// the surface. The diffuse target is gathered on a disc facing the camera, at the radii that
// split the farthest-reaching channel's profile into equal shares of its energy and at
// golden-angle turns (the set turned by the accumulation sample, so a still view converges).
// Each sample weighs, per channel, its profile at the 3D distance (the disc radius and the
// depth difference) over the sampling density, so a sample across a depth jump weighs almost
// nothing; the sum is normalized, so even light stays as it is. Blended with `own` by the
// share that scatters. Not gathered where the farthest radius is under a pixel.
fn scattered(pixel: vec2<f32>, p: vec3<f32>, own: vec3<f32>) -> vec3<f32> {
    let d = max(lit.scatter.rgb, vec3<f32>(1e-12));
    let far = max(d.r, max(d.g, d.b));
    let size = pixel_size(p);
    if lit.radii[5].x * far < size {
        return own;
    }
    let turn = fract(f32(f.counts.z) * 0.618034) * 2.0 * PI;
    var sum = vec3<f32>(0.0);
    var total = vec3<f32>(0.0);
    for (var i = 0u; i < SCATTER_SAMPLES; i++) {
        let r = lit.radii[i / 4u][i % 4u] * far;
        let phi = GOLDEN_ANGLE * f32(i) + turn;
        let at = vec2<i32>(floor(pixel + vec2<f32>(cos(phi), sin(phi)) * (r / size)));
        if !in_viewport(at) {
            continue;
        }
        let dq = textureLoad(depth, at, 0);
        if dq <= 0.0 {
            continue;
        }
        let dz = view_z(dq) - p.z;
        let w = burley(sqrt(r * r + dz * dz), d) / burley(r, vec3<f32>(far)).x;
        sum += w * textureLoad(diffuse_light, at, 0).rgb;
        total += w;
    }
    let gathered = select(own, sum / total, total > vec3<f32>(0.0));
    return own + (gathered - own) * lit.scatter.w;
}

// The diffuse pass of live subsurface: each drawn pixel's diffuse irradiance, for the display
// pass to gather.
@fragment fn fs_diffuse(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(floor(pos.xy));
    let d = textureLoad(depth, at, 0);
    if d <= 0.0 {
        return vec4<f32>(0.0);
    }
    let n = oct_decode(textureLoad(surface, at, 0).xy);
    let p = view_point(pos.xy, view_z(d));
    let albedo = lit.base.rgb * paint_at(at);
    return vec4<f32>(lit_irradiance(n, visibility(pos.xy, p, n), albedo), 1.0);
}
