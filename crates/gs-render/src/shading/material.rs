//! The one BRDF every part of the program evaluates from a material's physical numbers
//! (gs-core's [`Material`], `OpenPBR`'s names, research 11, 12.1; decision 0043 moved the
//! numbers to gs-core and left their evaluation here).
//!
//! Layers, as `OpenPBR` stacks them: a base that mixes a dielectric (specular lobe over a
//! diffuse or subsurface body) with a metal by `base_metalness`, under an optional clear coat.
//! Each lobe is GGX with height-correlated Smith visibility and energy compensation; what a
//! layer's specular reflects is taken from what lies under it, by its directional albedo
//! toward the viewer (`OpenPBR`'s albedo scaling, which is energy-conserving but not
//! reciprocal).

use super::Rgb;
use super::brdf::{Fresnel, alpha, ggx_lobe, lambert};
use super::dfg::Dfg;
use crate::math::{V3, dot};

pub use gs_core::material::{Color, Material};

/// Reflectance at normal incidence of a dielectric with index `ior` in air.
pub fn ior_to_f0(ior: f32) -> f32 {
    let r = (ior - 1.0) / (ior + 1.0);
    r * r
}

/// The BRDF a [`Material`] defines, and the pieces the MatCap baker and Lit mode take from it.
pub trait Brdf {
    /// The dielectric's Fresnel curve (`KHR_materials_specular`'s rule for weight and color).
    fn dielectric_fresnel(&self) -> Fresnel;

    /// The metal's Fresnel curve: base color at normal incidence, F82 tint toward grazing.
    fn metal_fresnel(&self) -> Fresnel;

    /// The coat's Fresnel curve.
    fn coat_fresnel(&self) -> Fresnel;

    /// The light each layer leaves to what lies under it, toward a viewer at cosine `mu`:
    /// `(diffuse weight of the dielectric, weight of the whole base under the coat)`. Both
    /// are one minus the directional albedo of the specular above, compensated.
    fn leftover(&self, mu: f32, compensate: bool) -> (Rgb, Rgb);

    /// The whole BRDF for unit vectors `n`, `v` (toward the viewer) and `l` (toward the
    /// light), with energy compensation, the body as Lambert. Not multiplied by `n.l`.
    fn brdf(&self, n: V3, v: V3, l: V3) -> Rgb;

    /// The same BRDF with single scattering only: each lobe loses the light that bounces
    /// between microfacets more than once, up to about half of it for rough metals.
    fn single_scattering_brdf(&self, n: V3, v: V3, l: V3) -> Rgb;
}

impl Brdf for Material {
    fn dielectric_fresnel(&self) -> Fresnel {
        let f0 = ior_to_f0(self.specular_ior);
        let w = self.specular_weight;
        let tinted = self.specular_color.0.map(|c| (f0 * c).min(1.0) * w);
        Fresnel::schlick(tinted, [w; 3])
    }

    fn metal_fresnel(&self) -> Fresnel {
        let w = self.specular_weight;
        Fresnel::f82(
            self.base_color.0.map(|c| c * w),
            [w; 3],
            self.specular_color.0,
        )
    }

    fn coat_fresnel(&self) -> Fresnel {
        Fresnel::schlick([ior_to_f0(self.coat_ior); 3], [1.0; 3])
    }

    fn leftover(&self, mu: f32, compensate: bool) -> (Rgb, Rgb) {
        let dfg = Dfg::get();
        let albedo = |f: &Fresnel, roughness: f32| {
            let a = dfg.at(mu, roughness);
            if compensate {
                a.compensated(f)
            } else {
                a.with(f)
            }
        };
        let diffuse = albedo(&self.dielectric_fresnel(), self.specular_roughness).map(|e| 1.0 - e);
        let coat = albedo(&self.coat_fresnel(), self.coat_roughness);
        let under = std::array::from_fn(|k| {
            (1.0 - self.coat_weight * coat[k])
                * (1.0 - self.coat_weight * (1.0 - self.coat_color.0[k]))
        });
        (diffuse, under)
    }

    fn brdf(&self, n: V3, v: V3, l: V3) -> Rgb {
        evaluate(self, (n, v, l), true)
    }

    fn single_scattering_brdf(&self, n: V3, v: V3, l: V3) -> Rgb {
        evaluate(self, (n, v, l), false)
    }
}

fn evaluate(m: &Material, (n, v, l): (V3, V3, V3), compensate: bool) -> Rgb {
    let mu = dot(n, v).max(0.0);
    let dfg = Dfg::get();
    let lobe = |f: &Fresnel, roughness: f32| {
        let comp = dfg.at(mu, roughness).compensation(f.f0);
        let raw = ggx_lobe(f, alpha(roughness), n, v, l);
        std::array::from_fn::<f32, 3, _>(|k| if compensate { raw[k] * comp[k] } else { raw[k] })
    };
    let (diffuse_weight, under) = m.leftover(mu, compensate);
    let body = lambert(m.base_color.0, n, v, l);
    let dielectric = lobe(&m.dielectric_fresnel(), m.specular_roughness);
    let metal = lobe(&m.metal_fresnel(), m.specular_roughness);
    let coat = lobe(&m.coat_fresnel(), m.coat_roughness);
    let metalness = m.base_metalness;
    std::array::from_fn(|k| {
        let d = dielectric[k] + diffuse_weight[k] * body[k];
        let base = d * (1.0 - metalness) + metal[k] * metalness;
        m.coat_weight * coat[k] + under[k] * base
    })
}
