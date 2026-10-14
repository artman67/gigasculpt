//! The MatCap bake against direct integration: what the baker stores at a pixel is the
//! material's BRDF integrated against the environment for that pixel's normal.

use crate::{narrow, wide};
use gs_render::matcap::{Bake, Options, Recipe, bake_under};
use gs_render::shading::brdf::alpha;
use gs_render::shading::{Brdf, Color, Environment, Material};
use gs_testkit::reference::shading::{self as reference, Rgb, V, dot, unit};

/// A smooth sky: a broad glow over a gradient.
fn sky(d: V) -> Rgb {
    let sun = unit([-0.4, 0.7, 0.6]);
    let glow = libm::pow(dot(d, sun).max(0.0), 6.0);
    [
        0.25 + 1.5 * glow,
        0.3 + 1.4 * glow + 0.05 * d[1],
        0.4 + glow,
    ]
}

/// The light leaving normal `n` toward +z: `integral of f(n, v, l) L(l) n.l dl`, with `L` the
/// sky metered as an environment meters it.
fn reference_pixel(m: &Material, n: V, scale: f64) -> Rgb {
    let v = [0.0, 0.0, 1.0];
    let a = f64::from(alpha(m.specular_roughness));
    reference::hemisphere(n, v, a, 128, &|l| {
        let f = wide(m.brdf(narrow(n), narrow(v), narrow(l)));
        let light = sky(l);
        std::array::from_fn(|k| f[k] * light[k] * scale * dot(n, l))
    })
}

/// At sampled pixels of four looks (a rough gray and a satin red dielectric, a measured metal,
/// a coated dielectric) the bake's `albedo x D + S` is the direct integral within 3%.
#[test]
fn a_bake_is_its_brdf_integrated_against_the_environment() {
    let env = Environment::from_light("sky", |d| narrow(sky(wide(d))));
    let directions = reference::sphere(128);
    let facing = reference::irradiance(&directions, &sky, [0.0, 0.0, 1.0]);
    let scale =
        std::f64::consts::PI / (0.2126 * facing[0] + 0.7152 * facing[1] + 0.0722 * facing[2]);
    let looks = [
        Material {
            base_color: Color([0.5, 0.5, 0.5]),
            specular_roughness: 0.6,
            ..Material::default()
        },
        Material {
            base_color: Color([0.6, 0.1, 0.05]),
            specular_roughness: 0.35,
            ..Material::default()
        },
        Material {
            base_color: Color([0.654, 0.685, 0.701]),
            specular_color: Color([0.688, 0.728, 0.798]),
            base_metalness: 1.0,
            specular_roughness: 0.4,
            ..Material::default()
        },
        Material {
            base_color: Color([0.3, 0.4, 0.2]),
            specular_roughness: 0.7,
            coat_weight: 1.0,
            coat_roughness: 0.3,
            ..Material::default()
        },
    ];
    let size = 32u32;
    for m in looks {
        let recipe = Recipe {
            name: "test".into(),
            catalog_id: String::new(),
            environment: "studio".into(),
            material: m.clone(),
            bake: Bake {
                size,
                ..Bake::default()
            },
            ..Recipe::default()
        };
        let baked = bake_under(&recipe, &env, Options::default());
        for (x, y) in [
            (16, 16),
            (8, 10),
            (24, 9),
            (10, 24),
            (22, 22),
            (16, 4),
            (4, 16),
        ] {
            let u = (f64::from(x) + 0.5) / 16.0 - 1.0;
            let w = (f64::from(y) + 0.5) / 16.0 - 1.0;
            let n = [u, -w, (1.0 - u * u - w * w).sqrt()];
            let want = reference_pixel(&m, n, scale);
            let got = wide(baked.radiance(x, y));
            for k in 0..3 {
                assert!(
                    (got[k] - want[k]).abs() <= 0.03 * want[k].max(0.05),
                    "{m:?} pixel ({x}, {y}): {got:?} against {want:?}"
                );
            }
        }
    }
}
