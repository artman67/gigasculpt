//! The microfacet lobes (research 11, section 2): GGX distribution, height-correlated Smith
//! visibility, Schlick Fresnel with the F82 tint for metals, and Lambert. The formulas are the
//! ones Filament and glTF Appendix B write down; `OpenPBR`'s F82 term is Kutz 2021's.

use super::Rgb;
use crate::math::{V3, dot, normalize_or, scale, sub};
use std::f32::consts::PI;

/// The smallest GGX alpha used: below it the lobe is a mirror in every way that shows, and
/// the distribution's peak would overflow.
pub const MIN_ALPHA: f32 = 1e-4;

/// GGX alpha for a perceptual roughness: roughness squared (Burley 2012).
pub fn alpha(roughness: f32) -> f32 {
    (roughness * roughness).max(MIN_ALPHA)
}

/// The GGX (Trowbridge-Reitz) normal distribution at `n_h` = cos of the half-vector angle.
pub fn ggx(n_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = n_h * n_h * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d)
}

/// Height-correlated Smith visibility, `G2 / (4 n.v n.l)` (Heitz 2014, in Filament's form).
pub fn smith_visibility(n_v: f32, n_l: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let gv = n_l * (n_v * n_v * (1.0 - a2) + a2).sqrt();
    let gl = n_v * (n_l * n_l * (1.0 - a2) + a2).sqrt();
    0.5 / (gv + gl)
}

/// `(1 - c)^5`, Schlick's weight.
pub fn schlick_weight(c: f32) -> f32 {
    let m = (1.0 - c).clamp(0.0, 1.0);
    let m2 = m * m;
    m2 * m2 * m
}

/// `c (1 - c)^6`, the shape of the F82 dip.
pub fn f82_weight(c: f32) -> f32 {
    let m = (1.0 - c).clamp(0.0, 1.0);
    let m3 = m * m * m;
    c * m3 * m3
}

/// cos 82 degrees, rounded to 1/7 as in Kutz 2021 and `OpenPBR`.
const MU_82: f32 = 1.0 / 7.0;

/// A Fresnel curve: Schlick from `f0` at normal incidence to `f90` at grazing, minus the F82
/// dip `b c (1 - c)^6` that makes metals darken just before grazing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fresnel {
    pub f0: Rgb,
    pub f90: Rgb,
    pub b: Rgb,
}

impl Fresnel {
    /// Plain Schlick.
    pub fn schlick(f0: Rgb, f90: Rgb) -> Fresnel {
        Fresnel {
            f0,
            f90,
            b: [0.0; 3],
        }
    }

    /// The F82-tint model (`OpenPBR`): Schlick through `f0`, scaled at cos = 1/7 by `tint`.
    pub fn f82(f0: Rgb, f90: Rgb, tint: Rgb) -> Fresnel {
        let w = schlick_weight(MU_82);
        let at = |k: usize| f0[k] + (f90[k] - f0[k]) * w;
        let b = std::array::from_fn(|k| at(k) * (1.0 - tint[k]) / f82_weight(MU_82));
        Fresnel { f0, f90, b }
    }

    /// The reflectance at `c` = cos of the angle to the (micro)normal, at most 1.
    pub fn at(&self, c: f32) -> Rgb {
        let (s, f) = (schlick_weight(c), f82_weight(c));
        std::array::from_fn(|k| {
            (self.f0[k] + (self.f90[k] - self.f0[k]) * s - self.b[k] * f).clamp(0.0, 1.0)
        })
    }
}

/// A raw GGX specular lobe, `F D V`, for unit vectors `n`, `v` and `l`; zero below the
/// horizon. Reciprocal in `v` and `l`.
pub fn ggx_lobe(fresnel: &Fresnel, alpha: f32, n: V3, v: V3, l: V3) -> Rgb {
    let (n_v, n_l) = (dot(n, v), dot(n, l));
    if n_v <= 0.0 || n_l <= 0.0 {
        return [0.0; 3];
    }
    let h = normalize_or(crate::math::add(v, l), n);
    let k = ggx(dot(n, h).max(0.0), alpha) * smith_visibility(n_v, n_l, alpha);
    fresnel.at(dot(v, h).max(0.0)).map(|f| f * k)
}

/// Lambert's diffuse lobe for `albedo`: `albedo / pi` above the horizon on both sides.
pub fn lambert(albedo: Rgb, n: V3, v: V3, l: V3) -> Rgb {
    if dot(n, v) <= 0.0 || dot(n, l) <= 0.0 {
        return [0.0; 3];
    }
    albedo.map(|a| a / PI)
}

/// Two unit vectors completing `n` to a right-handed orthonormal frame (Duff et al. 2017,
/// the construction Blender's MatCap lookup uses too).
pub fn frame(n: V3) -> (V3, V3) {
    let sign = if n[2] >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (sign + n[2]);
    let b = n[0] * n[1] * a;
    let t = [1.0 + sign * n[0] * n[0] * a, sign * b, -sign * n[0]];
    let s = [b, sign + n[1] * n[1] * a, -n[1]];
    (t, s)
}

/// The `i`th of `count` points of the Hammersley set in the unit square.
pub fn hammersley(i: u32, count: u32) -> [f32; 2] {
    let bits = i.reverse_bits();
    [
        (i as f32 + 0.5) / count as f32,
        bits as f32 * (1.0 / 4_294_967_296.0),
    ]
}

/// A GGX microfacet normal around `n` for the point `u` of the unit square, distributed as
/// `D(h) n.h`, and that density.
pub fn sample_ggx(n: V3, alpha: f32, u: [f32; 2]) -> (V3, f32) {
    let a2 = alpha * alpha;
    let cos2 = ((1.0 - u[0]) / (1.0 + (a2 - 1.0) * u[0])).clamp(0.0, 1.0);
    let (cos, sin) = (cos2.sqrt(), (1.0 - cos2).sqrt());
    let (sp, cp) = libm::sincosf(2.0 * PI * u[1]);
    let (t, s) = frame(n);
    let h = std::array::from_fn(|k| t[k] * sin * cp + s[k] * sin * sp + n[k] * cos);
    (h, ggx(cos, alpha) * cos)
}

/// Smith's masking for GGX toward a direction at cosine `c`:
/// `2 c / (c + sqrt(alpha^2 + (1 - alpha^2) c^2))`.
pub fn smith_masking(c: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    2.0 * c / (c + (a2 + (1.0 - a2) * c * c).sqrt()).max(1e-12)
}

/// A GGX microfacet normal around `n` that viewer `v` sees, for the point `u` of the unit
/// square (Heitz 2018, sampling the visible normals): distributed as `G1(v) v.h D(h) / n.v`,
/// so a reflected direction has density `G1(v) D / (4 n.v)` and the lobe's estimator weight
/// is just `F G2 / G1(v)`. Far steadier than [`sample_ggx`] toward grazing views.
pub fn sample_visible(n: V3, v: V3, alpha: f32, u: [f32; 2]) -> V3 {
    let (t, s) = frame(n);
    // The view in the frame of n, stretched to the unit-roughness configuration.
    let local = [dot(v, t), dot(v, s), dot(v, n).max(1e-6)];
    let vh = normalize_or(
        [alpha * local[0], alpha * local[1], local[2]],
        [0.0, 0.0, 1.0],
    );
    let len2 = vh[0] * vh[0] + vh[1] * vh[1];
    let t1 = if len2 > 0.0 {
        let inv = 1.0 / len2.sqrt();
        [-vh[1] * inv, vh[0] * inv, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t2 = crate::math::cross(vh, t1);
    let r = u[0].sqrt();
    let (sp, cp) = libm::sincosf(2.0 * PI * u[1]);
    let p1 = r * cp;
    let blend = f32::midpoint(1.0, vh[2]);
    let p2 = (1.0 - blend) * (1.0 - p1 * p1).max(0.0).sqrt() + blend * r * sp;
    let p3 = (1.0 - p1 * p1 - p2 * p2).max(0.0).sqrt();
    let nh: V3 = std::array::from_fn(|k| p1 * t1[k] + p2 * t2[k] + p3 * vh[k]);
    let h = normalize_or(
        [alpha * nh[0], alpha * nh[1], nh[2].max(0.0)],
        [0.0, 0.0, 1.0],
    );
    std::array::from_fn(|k| t[k] * h[0] + s[k] * h[1] + n[k] * h[2])
}

/// `v` mirrored about `h`.
pub fn reflect(v: V3, h: V3) -> V3 {
    sub(scale(h, 2.0 * dot(v, h)), v)
}
