//! Integrals of a GGX lobe against an environment by filtered importance sampling (Colbert and
//! Krivanek, GPU Gems 3 chapter 20): each GGX sample reads the radiance cube at the mip level
//! whose texels cover about the solid angle that sample stands for, so a fixed set of a few
//! dozen samples gives a smooth result at every roughness.

use super::Rgb;
use super::brdf::{Fresnel, alpha, ggx, hammersley, reflect, sample_ggx, sample_visible};
use super::brdf::{smith_masking, smith_visibility};
use super::environment::Environment;
use crate::math::{V3, dot};
use std::f32::consts::PI;

/// The mip level for a sample of density `pdf` (per steradian) among `count` samples: the
/// level whose texels cover the solid angle the sample stands for. Colbert and Krivanek add
/// one level on top; measured against direct integration that blurs 3 to 6% too far at 64
/// samples, so it is left out.
fn lod(env: &Environment, pdf: f32, count: u32) -> f32 {
    let size = env.radiance.size() as f32;
    let texel = 4.0 * PI / (6.0 * size * size);
    let sample = 1.0 / (count as f32 * pdf.max(1e-12));
    (0.5 * libm::log2f(sample / texel)).max(0.0)
}

/// The radiance a GGX lobe of `roughness` around normal `n` gathers toward `v`, weighted by
/// each Fresnel curve in `fresnels`: for each, the mean environment radiance under the weight
/// `F D V n.l`, from `count` fixed samples. Times the lobe's directional albedo (from the DFG
/// table) this is the light the lobe reflects; dividing it out this way (a ratio estimator)
/// keeps the albedo exact, so a uniform environment comes back exactly.
pub fn gather<const K: usize>(
    env: &Environment,
    (n, v): (V3, V3),
    roughness: f32,
    fresnels: &[Fresnel; K],
    count: u32,
) -> [Rgb; K] {
    let a = alpha(roughness);
    let n_v = dot(n, v).max(1e-4);
    let g1 = smith_masking(n_v, a);
    let mut light = [[0.0f32; 3]; K];
    let mut weight = [[0.0f32; 3]; K];
    for i in 0..count {
        let h = sample_visible(n, v, a, hammersley(i, count));
        let l = reflect(v, h);
        let (n_l, v_h) = (dot(n, l), dot(v, h));
        if n_l <= 0.0 || v_h <= 0.0 {
            continue;
        }
        let pdf_l = g1 * ggx(dot(n, h).max(0.0), a) / (4.0 * n_v);
        let seen = env.radiance(l, lod(env, pdf_l, count));
        // G2 / G1(v): the visible-normal estimator's weight before Fresnel.
        let k = 4.0 * smith_visibility(n_v, n_l, a) * n_v * n_l / g1;
        for (f, (sum, w)) in fresnels.iter().zip(light.iter_mut().zip(weight.iter_mut())) {
            let fr = f.at(v_h);
            for c in 0..3 {
                sum[c] += fr[c] * k * seen[c];
                w[c] += fr[c] * k;
            }
        }
    }
    std::array::from_fn(|j| std::array::from_fn(|c| light[j][c] / weight[j][c].max(1e-12)))
}

/// GGX-prefiltered radiance along `r` for `roughness` (Karis 2013): the lobe with normal and
/// view along `r`, samples weighted by `n.l`. What one mip of a live lit mode's prefiltered
/// cube holds; the split sum then multiplies it by the DFG albedo.
pub fn prefiltered(env: &Environment, r: V3, roughness: f32, count: u32) -> Rgb {
    let a = alpha(roughness);
    let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
    for i in 0..count {
        let (h, _) = sample_ggx(r, a, hammersley(i, count));
        let l = reflect(r, h);
        let n_l = dot(r, l);
        if n_l <= 0.0 {
            continue;
        }
        let n_h = dot(r, h).max(1e-6);
        // With n = v the density of l is D n.h / (4 v.h) = D / 4.
        let seen = env.radiance(l, lod(env, ggx(n_h, a) / 4.0, count));
        for c in 0..3 {
            sum[c] += seen[c] * n_l;
        }
        total += n_l;
    }
    sum.map(|s| s / total.max(1e-12))
}
