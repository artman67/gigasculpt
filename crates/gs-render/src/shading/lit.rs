//! Lit mode on the CPU (decision 0023): a material lit live by an environment and disc lights,
//! exactly as the display pass's Lit branch computes it, so it is the reference that branch is
//! checked against.
//!
//! The model is the MatCap bake's (`matcap::bake`): the dielectric, metal and coat albedos from
//! the DFG table with Filament's compensation, the body's share `1 - E` (`OpenPBR`'s albedo
//! scaling), the coat over the base. Two established real-time approximations replace the
//! bake's integrals:
//!
//! - diffuse: the environment's SH9 irradiance (Ramamoorthi and Hanrahan 2001);
//! - specular: the split sum (Karis 2013): the environment prefiltered with `n = v = r` at a
//!   few roughness steps, read along Filament's dominant direction `mix(r, n, a^2)` at
//!   Filament's level `(levels - 1) r (2 - r)`, times the DFG albedo.
//!
//! Before either, light 1 is taken out of the environment ([`super::light::extract`]) and lit
//! as a disc. Occlusion darkens environment light only: GTAO's multi-bounce fit on the
//! diffuse, Lagarde and de Rousiers' specular occlusion on the reflections.

use super::Rgb;
use super::brdf::{Fresnel, alpha, ggx_lobe, reflect};
use super::cube::{Cube, Face, direction};
use super::dfg::Dfg;
use super::environment::{Environment, SHIPPED};
use super::light::{DiscLight, extract};
use super::lobe::prefiltered;
use super::material::{Brdf, Material};
use crate::error::RenderError;
use crate::math::{V3, dot, normalize_or};
use std::f32::consts::PI;
use std::sync::{Arc, OnceLock};

/// Mip levels of the prefiltered cube: 128 down to 8 texels per face, as Filament stops at 16
/// of 256. Level `i` holds roughness [`level_roughness`]`(i)`.
pub const PREFILTER_LEVELS: usize = 5;
/// GGX samples per texel of the prefiltered cube (filtered importance sampling).
const PREFILTER_SAMPLES: u32 = 64;

/// The roughness mip `level` of the prefiltered cube holds: the inverse of [`roughness_lod`].
pub fn level_roughness(level: usize) -> f32 {
    let t = (level as f32 / (PREFILTER_LEVELS - 1) as f32).clamp(0.0, 1.0);
    1.0 - (1.0 - t).sqrt()
}

/// The prefiltered cube's level for perceptual `roughness`: Filament's
/// `(levels - 1) r (2 - r)`, which gives low roughness, where lobes change fastest, more of
/// the levels.
pub fn roughness_lod(roughness: f32) -> f32 {
    let r = roughness.clamp(0.0, 1.0);
    (PREFILTER_LEVELS - 1) as f32 * r * (2.0 - r)
}

/// An environment ready for Lit mode: light 1 taken out, the rest with its SH fit and its
/// GGX-prefiltered cube.
#[derive(Clone, Debug, PartialEq)]
pub struct LitEnvironment {
    pub name: String,
    /// The environment without light 1 (what Ambient scales).
    pub rest: Environment,
    /// The rest prefiltered for GGX lobes, level `i` at roughness [`level_roughness`]`(i)`.
    pub prefiltered: Cube,
    /// Light 1 as the extraction found it; `None` when nothing is brighter than the threshold.
    pub key: Option<DiscLight>,
}

impl LitEnvironment {
    /// Takes light 1 out of `env` and prepares the rest. Seconds for a 128 cube; cache it.
    pub fn of(env: &Environment) -> LitEnvironment {
        let (radiance, key) = match extract(&env.radiance) {
            Some((rest, key)) => (rest, Some(key)),
            None => (env.radiance.clone(), None),
        };
        let rest = Environment::derived(&env.name, radiance);
        LitEnvironment {
            name: env.name.clone(),
            prefiltered: prefilter(&rest),
            rest,
            key,
        }
    }

    /// A shipped environment prepared for Lit mode, built once per process.
    pub fn shipped(name: &str) -> Result<Arc<LitEnvironment>, RenderError> {
        static BUILT: [OnceLock<Arc<LitEnvironment>>; SHIPPED.len()] =
            [const { OnceLock::new() }; SHIPPED.len()];
        let env = Environment::shipped(name)?;
        let at = SHIPPED.iter().position(|(n, _)| *n == name).unwrap_or(0);
        Ok(Arc::clone(
            BUILT[at].get_or_init(|| Arc::new(LitEnvironment::of(&env))),
        ))
    }
}

/// The GGX-prefiltered cube of `env`: level 0 its sharpest radiance (roughness 0 is a mirror),
/// each later level half the size at the next roughness step.
fn prefilter(env: &Environment) -> Cube {
    let mut levels = vec![env.radiance.levels[0].clone()];
    for level in 1..PREFILTER_LEVELS {
        let size = (env.radiance.size() >> level).max(1);
        let roughness = level_roughness(level);
        levels.push(std::array::from_fn(|face| {
            let texels = gs_par::map(size * size, |t| {
                let (i, j) = (t % size, t / size);
                let (u, v) = (
                    (i as f32 + 0.5) / size as f32,
                    (j as f32 + 0.5) / size as f32,
                );
                prefiltered(env, direction(face, u, v), roughness, PREFILTER_SAMPLES)
            });
            Face { size, texels }
        }));
    }
    Cube { levels }
}

/// GTAO's multi-bounce fit (Jimenez and colleagues 2016): visibility `a` raised for light
/// bouncing between nearby surfaces of albedo `r`, never below `a`. As `lighting.wgsl`.
pub fn multi_bounce(a: f32, r: Rgb) -> Rgb {
    r.map(|r| {
        let (x, y, z) = (
            2.0404 * r - 0.3324,
            0.6417 - 4.7951 * r,
            2.7552 * r + 0.6903,
        );
        a.max(((x * a + y) * a + z) * a)
    })
}

/// Specular occlusion from visibility `a` for a lobe of GGX `alpha` seen at `n.v` (Lagarde and
/// de Rousiers 2014, as Filament). As `lighting.wgsl`.
pub fn specular_occlusion(n_v: f32, a: f32, alpha: f32) -> f32 {
    (libm::powf(n_v + a, libm::exp2f(-16.0 * alpha - 1.0)) - 1.0 + a).clamp(0.0, 1.0)
}

/// The light around a point in Lit mode.
#[derive(Clone, Copy, Debug)]
pub struct Lighting<'a> {
    pub environment: &'a LitEnvironment,
    /// The lights that are on, light 1 first when it is.
    pub lights: &'a [DiscLight],
    /// Scales the environment's light.
    pub ambient: f32,
}

/// The light leaving one point, split as live subsurface needs it: the diffuse is
/// `weight x irradiance`, and subsurface spreads the irradiance before the weight applies.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shaded {
    /// Irradiance reaching the body: the environment's (occluded) and the lights'.
    pub irradiance: Rgb,
    /// What turns irradiance into diffuse radiance: the body's share of what the layers above
    /// leave, times albedo over pi.
    pub weight: Rgb,
    /// Everything the specular layers reflect.
    pub specular: Rgb,
}

impl Shaded {
    /// The radiance toward the viewer.
    pub fn total(&self) -> Rgb {
        std::array::from_fn(|k| self.weight[k] * self.irradiance[k] + self.specular[k])
    }
}

/// Unit vector `a` blended toward unit `b` by `t`, normalized.
fn toward(a: V3, b: V3, t: f32) -> V3 {
    normalize_or(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t), b)
}

/// Material `m` at unit normal `n` seen from unit `v` (toward the viewer), with visibility `a`
/// (1: nothing occludes it), under `lit`.
pub fn shade(m: &Material, lit: &Lighting, n: V3, v: V3, a: f32) -> Shaded {
    let mu = dot(n, v).max(1e-4);
    let dfg = Dfg::get();
    let fresnels = [m.dielectric_fresnel(), m.metal_fresnel(), m.coat_fresnel()];
    let roughness = [m.specular_roughness, m.specular_roughness, m.coat_roughness];
    let albedo: [Rgb; 3] =
        std::array::from_fn(|i| dfg.at(mu, roughness[i]).compensated(&fresnels[i]));
    let (body, under) = m.leftover(mu, true);
    let metal = m.base_metalness;
    // How much each lobe counts: the dielectric and metal under the coat, the coat on top.
    let share: [Rgb; 3] = [
        under.map(|u| u * (1.0 - metal)),
        under.map(|u| u * metal),
        [m.coat_weight; 3],
    ];
    let base = m.base_color.0;
    let weight = std::array::from_fn(|k| share[0][k] * body[k] * base[k] / PI);
    let r = reflect(v, n);
    let env = lit.environment;

    // The environment: SH9 irradiance and the split sum, occluded.
    let bounce = multi_bounce(a, base);
    let sh = env.rest.sh9.irradiance(n);
    let mut irradiance: Rgb = std::array::from_fn(|k| sh[k] * lit.ambient * bounce[k]);
    // The dielectric and metal share one roughness and so one prefiltered read.
    let reflected = |rough: f32| {
        let al = alpha(rough);
        let seen = env
            .prefiltered
            .sample(toward(r, n, al * al), roughness_lod(rough));
        let so = specular_occlusion(mu, a, al) * lit.ambient;
        seen.map(|s| s * so)
    };
    let seen = reflected(m.specular_roughness);
    let mut specular: Rgb = std::array::from_fn(|k| {
        (share[0][k] * albedo[0][k] + share[1][k] * albedo[1][k]) * seen[k]
    });
    if m.coat_weight > 0.0 {
        let seen = reflected(m.coat_roughness);
        for k in 0..3 {
            specular[k] += share[2][k] * albedo[2][k] * seen[k];
        }
    }

    // The lights: the cap's exact irradiance, and each lobe through the representative point.
    for disc in lit.lights {
        let e = disc.irradiance(n);
        for k in 0..3 {
            irradiance[k] += e[k];
        }
        let l = disc.representative(r);
        let n_l = dot(n, l);
        if n_l <= 0.0 {
            continue;
        }
        for i in 0..3 {
            if share[i].iter().all(|&s| s <= 0.0) {
                continue;
            }
            let lobe = lobe_light(&fresnels[i], roughness[i], mu, (n, v, l));
            let k_disc = disc.normalization(alpha(roughness[i])) * n_l;
            for k in 0..3 {
                specular[k] += share[i][k] * lobe[k] * k_disc * disc.illuminance[k];
            }
        }
    }
    Shaded {
        irradiance,
        weight,
        specular,
    }
}

/// One GGX lobe toward light `l`, `F D V` with Filament's compensation for its albedo at `mu`.
fn lobe_light(f: &Fresnel, roughness: f32, mu: f32, (n, v, l): (V3, V3, V3)) -> Rgb {
    let raw = ggx_lobe(f, alpha(roughness), n, v, l);
    let compensation = Dfg::get().at(mu, roughness).compensation(f.f0);
    std::array::from_fn(|k| raw[k] * compensation[k])
}
