//! The MatCap baker (research 11, 12.3): a look's material on a ball under its environment,
//! seen from the view direction, as two linear HDR layers.
//!
//! - **D**, the diffuse response per unit albedo: irradiance at the normal (or, for a
//!   scattering material, irradiance carried along the ball's surface by Burley's profile),
//!   times what the specular layers above leave to the body, over pi.
//! - **S**, everything else: each specular lobe's light from the environment by filtered
//!   importance sampling, scaled to the lobe's exact compensated albedo from the DFG table.
//!
//! Drawn, a pixel is `albedo x D + S`. Deterministic: fixed sample sets, `libm`, and
//! `gs-par`'s fixed chunks, so a look bakes to the same bytes at any thread count.

use super::{Baked, Recipe};
use crate::error::RenderError;
use crate::math::{V3, normalize_or};
use crate::shading::brdf::frame;
use crate::shading::burley::{self, radii};
use crate::shading::cube::{Cube, Face, direction};
use crate::shading::dfg::Dfg;
use crate::shading::environment::Environment;
use crate::shading::irradiance::IRRADIANCE_SIZE;
use crate::shading::lobe::gather;
use crate::shading::material::{Brdf, Material};
use crate::shading::{Rgb, half_rgba};
use std::f32::consts::PI;
use std::sync::{Arc, Mutex, PoisonError};

/// Below this roughness reflections are sharp enough that a pixel is sampled 2 by 2.
const SHARP: f32 = 0.3;

/// GGX samples per lobe per pixel sample. A narrow lobe needs fewer: its samples read fine
/// mip levels that already cover its spread, and sharp lobes are also sampled 2 by 2 per
/// pixel. 64 for broad lobes keeps filtered importance sampling within about 2% of direct
/// integration (`gs-testkit` tests/shading).
fn lobe_samples(roughness: f32) -> u32 {
    if roughness < 0.1 {
        16
    } else if roughness < SHARP {
        32
    } else {
        64
    }
}

/// Switches for checking the model; shipped looks bake with the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Filament's multiple-scattering compensation. Off, rough lobes lose light, which the
    /// furnace test uses to show it bites.
    pub energy_compensation: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            energy_compensation: true,
        }
    }
}

/// The look's MatCap under its shipped environment, baked once per process and kept, by what
/// it depends on ([`Recipe::bake_key`]), so a retitled look is not baked again. Bakes take
/// turns (each one already uses every core), so two callers never bake the same look.
pub fn bake(recipe: &Recipe) -> Result<Arc<Baked>, RenderError> {
    static CACHE: Mutex<Vec<(String, Arc<Baked>)>> = Mutex::new(Vec::new());
    const KEEP: usize = 32;
    let key = format!("{:?}", recipe.bake_key());
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, baked)) = cache.iter().find(|(k, _)| *k == key) {
        return Ok(Arc::clone(baked));
    }
    let env = Environment::shipped(&recipe.environment)?;
    let baked = Arc::new(bake_under(recipe, &env, Options::default()));
    if cache.len() == KEEP {
        cache.remove(0);
    }
    cache.push((key, Arc::clone(&baked)));
    Ok(baked)
}

/// The sphere normal at image position (u, v) in -1..1, v down; past the silhouette, the
/// silhouette's.
fn sphere_normal(u: f32, v: f32) -> V3 {
    let z = (1.0 - u * u - v * v).max(0.0).sqrt();
    normalize_or([u, -v, z], [0.0, 0.0, 1.0])
}

/// The look's MatCap under `env`.
pub fn bake_under(recipe: &Recipe, env: &Environment, options: Options) -> Baked {
    let size = recipe.bake.size;
    let m = &recipe.material;
    let scatter = (m.subsurface_weight > 0.0).then(|| scattered(env, m, recipe.bake.sphere_radius));
    let sharp = m.specular_roughness < SHARP || (m.coat_weight > 0.0 && m.coat_roughness < SHARP);
    let pixel = 2.0 / size as f32;
    let layers = gs_par::map((size * size) as usize, |p| {
        let (x, y) = ((p as u32 % size) as f32, (p as u32 / size) as f32);
        let (u, v) = ((x + 0.5) * pixel - 1.0, (y + 0.5) * pixel - 1.0);
        let edge = ((u * u + v * v).sqrt() - 1.0).abs() < 2.0 * pixel;
        let offsets: &[[f32; 2]] = if sharp || edge {
            &[[-0.25, -0.25], [0.25, -0.25], [-0.25, 0.25], [0.25, 0.25]]
        } else {
            &[[0.0, 0.0]]
        };
        let mut sum = ([0.0f32; 3], [0.0f32; 3]);
        for [du, dv] in offsets {
            let n = sphere_normal(u + du * pixel, v + dv * pixel);
            let (d, s) = shade(m, env, scatter.as_ref(), n, options);
            for k in 0..3 {
                sum.0[k] += d[k];
                sum.1[k] += s[k];
            }
        }
        let count = offsets.len() as f32;
        (sum.0.map(|c| c / count), sum.1.map(|c| c / count))
    });
    Baked {
        size,
        d: layers.iter().map(|(d, _)| half_rgba(*d)).collect(),
        s: layers.iter().map(|(_, s)| half_rgba(*s)).collect(),
        albedo: m.base_color.0.map(f32::to_bits),
        roughness: m.specular_roughness.to_bits(),
        metalness: m.base_metalness.to_bits(),
    }
}

/// D and S for the ball's normal `n`, seen from +z.
fn shade(m: &Material, env: &Environment, scatter: Option<&Cube>, n: V3, o: Options) -> (Rgb, Rgb) {
    let v = [0.0, 0.0, 1.0];
    let mu = n[2].max(1e-4);
    let dfg = Dfg::get();
    let albedo = |f, roughness| {
        let a = dfg.at(mu, roughness);
        if o.energy_compensation {
            a.compensated(f)
        } else {
            a.with(f)
        }
    };
    let (dielectric, metal, coat) = (m.dielectric_fresnel(), m.metal_fresnel(), m.coat_fresnel());
    let [seen_d, seen_m] = gather(
        env,
        (n, v),
        m.specular_roughness,
        &[dielectric, metal],
        lobe_samples(m.specular_roughness),
    );
    let (e_d, e_m) = (
        albedo(&dielectric, m.specular_roughness),
        albedo(&metal, m.specular_roughness),
    );
    let (body, under) = m.leftover(mu, o.energy_compensation);
    let coat_light = if m.coat_weight > 0.0 {
        let [seen] = gather(
            env,
            (n, v),
            m.coat_roughness,
            &[coat],
            lobe_samples(m.coat_roughness),
        );
        let e = albedo(&coat, m.coat_roughness);
        std::array::from_fn(|k| m.coat_weight * e[k] * seen[k])
    } else {
        [0.0; 3]
    };
    let lit = env.irradiance(n);
    let irradiance: Rgb = match scatter {
        Some(cube) => {
            let carried = cube.sample(n, 0.0);
            std::array::from_fn(|k| lit[k] + (carried[k] - lit[k]) * m.subsurface_weight)
        }
        None => lit,
    };
    let metalness = m.base_metalness;
    let d = std::array::from_fn(|k| under[k] * (1.0 - metalness) * body[k] * irradiance[k] / PI);
    let s = std::array::from_fn(|k| {
        let base = (1.0 - metalness) * e_d[k] * seen_d[k] + metalness * e_m[k] * seen_m[k];
        coat_light[k] + under[k] * base
    });
    (d, s)
}

/// Irradiance carried under the surface of a ball of radius `ball` by Burley's profile, for
/// every normal of an irradiance-sized cube, per color channel. Each channel's profile is
/// sampled at fixed radii (its inverse CDF) and golden-angle turns, each sample moved that far
/// along the ball's surface and weighted by how the sphere's area differs from the plane's.
fn scattered(env: &Environment, m: &Material, ball: f32) -> Cube {
    let size = IRRADIANCE_SIZE;
    let shapes: Rgb = std::array::from_fn(|k| {
        let mfp = m.subsurface_radius * m.subsurface_radius_scale[k];
        burley::shape(mfp, m.base_color.0[k])
    });
    let level = std::array::from_fn(|face| {
        let texels = gs_par::map(size * size, |t| {
            let (i, j) = (t % size, t / size);
            let n = direction(
                face,
                (i as f32 + 0.5) / size as f32,
                (j as f32 + 0.5) / size as f32,
            );
            std::array::from_fn(|k| carried(env, n, shapes[k] / ball, k))
        });
        Face { size, texels }
    });
    Cube {
        levels: vec![level],
    }
}

/// Channel `k` of the irradiance carried to unit normal `n` on a unit ball by a profile of
/// shape `d` (in ball radii).
fn carried(env: &Environment, n: V3, d: f32, k: usize) -> f32 {
    const GOLDEN: f32 = 2.399_963;
    let (t, s) = frame(n);
    let (mut sum, mut total) = (0.0f32, 0.0f32);
    for (i, r) in radii().iter().enumerate() {
        let theta = r * d;
        if theta >= PI {
            continue;
        }
        let (sp, cp) = libm::sincosf(GOLDEN * i as f32);
        let (st, ct) = libm::sincosf(theta);
        let y: V3 = std::array::from_fn(|c| n[c] * ct + (t[c] * cp + s[c] * sp) * st);
        // Area on the sphere over area on the plane at this distance.
        let w = if theta > 1e-6 { st / theta } else { 1.0 };
        sum += w * env.irradiance(y)[k];
        total += w;
    }
    if total > 0.0 {
        sum / total
    } else {
        env.irradiance(n)[k]
    }
}
