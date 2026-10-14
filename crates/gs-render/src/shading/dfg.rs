//! The DFG table (Karis 2013, Filament section 5.3): the directional albedo of the GGX lobe,
//! split by Fresnel term, for every view angle and roughness. From it come the specular albedo
//! of any Fresnel curve, the energy compensation for multiple scattering (Filament's
//! `1 + F0 (1 / E - 1)`), and the share of light left for the diffuse base (`OpenPBR`'s albedo
//! scaling). Built once per process with a fixed sample set, so it is deterministic.

use super::Rgb;
use super::brdf::{Fresnel, alpha, f82_weight, hammersley, reflect, sample_visible};
use super::brdf::{schlick_weight, smith_masking, smith_visibility};
use crate::math::dot;
use std::sync::OnceLock;

/// Table size along the view cosine and along roughness.
pub const SIZE: usize = 64;
/// GGX samples per table entry.
const SAMPLES: u32 = 512;
/// The view cosine of the table's first column: exactly 0 has no view direction.
const MIN_MU: f32 = 1e-3;

/// The three integrals of the Fresnel-free GGX lobe `D V n.l` over the hemisphere: in full
/// (`total`), weighted by Schlick's `(1 - v.h)^5` (`schlick`) and by the F82 shape
/// `v.h (1 - v.h)^6` (`dip`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Albedo {
    pub total: f32,
    pub schlick: f32,
    pub dip: f32,
}

impl Albedo {
    /// The lobe's directional albedo with Fresnel curve `f`, before compensation.
    pub fn with(&self, f: &Fresnel) -> Rgb {
        std::array::from_fn(|k| {
            f.f0[k] * (self.total - self.schlick) + f.f90[k] * self.schlick - f.b[k] * self.dip
        })
    }

    /// Filament's multiple-scattering compensation for a lobe with normal reflectance `f0`.
    pub fn compensation(&self, f0: Rgb) -> Rgb {
        f0.map(|f| 1.0 + f * (1.0 / self.total.max(1e-4) - 1.0))
    }

    /// The compensated lobe's directional albedo with Fresnel curve `f`, at most 1.
    pub fn compensated(&self, f: &Fresnel) -> Rgb {
        let (e, c) = (self.with(f), self.compensation(f.f0));
        std::array::from_fn(|k| (e[k] * c[k]).clamp(0.0, 1.0))
    }
}

/// The three integrals for view cosine `mu` and roughness, by sampling the visible normals
/// with `samples` Hammersley points: what the table stores, computed directly.
pub fn integrate(mu: f32, roughness: f32, samples: u32) -> Albedo {
    let a = alpha(roughness);
    let mu = mu.clamp(MIN_MU, 1.0);
    let v = [(1.0 - mu * mu).sqrt(), 0.0, mu];
    let n = [0.0, 0.0, 1.0];
    let g1 = smith_masking(mu, a);
    let mut sum = Albedo::default();
    for i in 0..samples {
        let h = sample_visible(n, v, a, hammersley(i, samples));
        let l = reflect(v, h);
        let (n_l, v_h) = (l[2], dot(v, h));
        if n_l <= 0.0 || v_h <= 0.0 {
            continue;
        }
        // D V n.l over the density G1 D / (4 n.v): G2 / G1, with G2 = 4 V n.v n.l.
        let k = 4.0 * smith_visibility(mu, n_l, a) * mu * n_l / g1;
        sum.total += k;
        sum.schlick += k * schlick_weight(v_h);
        sum.dip += k * f82_weight(v_h);
    }
    let s = samples as f32;
    Albedo {
        total: sum.total / s,
        schlick: sum.schlick / s,
        dip: sum.dip / s,
    }
}

/// The table, `SIZE` by `SIZE`: row `j` is roughness `j / (SIZE - 1)`, column `i` view cosine
/// `i / (SIZE - 1)` (the first at `MIN_MU`).
pub struct Dfg {
    entries: Vec<Albedo>,
}

impl Dfg {
    /// The table, built on first use. The build is serial on purpose: its first use can come
    /// from inside a parallel task (a pixel of a bake), and a build that waited on the same
    /// worker threads would deadlock them.
    pub fn get() -> &'static Dfg {
        static TABLE: OnceLock<Dfg> = OnceLock::new();
        TABLE.get_or_init(|| {
            let step = 1.0 / (SIZE - 1) as f32;
            let entries = (0..SIZE * SIZE)
                .map(|e| {
                    let (j, i) = (e / SIZE, e % SIZE);
                    integrate(i as f32 * step, j as f32 * step, SAMPLES)
                })
                .collect();
            Dfg { entries }
        })
    }

    /// The table as `SIZE` rows of `SIZE` texels `(total, schlick, dip, 0)`, row `j` roughness
    /// `j / (SIZE - 1)`: what Lit mode's shader reads and blends as [`Dfg::at`] does.
    pub fn texels(&self) -> Vec<[f32; 4]> {
        (self.entries.iter())
            .map(|a| [a.total, a.schlick, a.dip, 0.0])
            .collect()
    }

    /// The integrals at view cosine `mu` and roughness, interpolated bilinearly.
    pub fn at(&self, mu: f32, roughness: f32) -> Albedo {
        let x = mu.clamp(0.0, 1.0) * (SIZE - 1) as f32;
        let y = roughness.clamp(0.0, 1.0) * (SIZE - 1) as f32;
        let (i, j) = ((x as usize).min(SIZE - 2), (y as usize).min(SIZE - 2));
        let (fx, fy) = (x - i as f32, y - j as f32);
        let e = |j: usize, i: usize| self.entries[j * SIZE + i];
        let mix = |a: Albedo, b: Albedo, t: f32| Albedo {
            total: a.total + (b.total - a.total) * t,
            schlick: a.schlick + (b.schlick - a.schlick) * t,
            dip: a.dip + (b.dip - a.dip) * t,
        };
        let lo = mix(e(j, i), e(j, i + 1), fx);
        let hi = mix(e(j + 1, i), e(j + 1, i + 1), fx);
        mix(lo, hi, fy)
    }
}
