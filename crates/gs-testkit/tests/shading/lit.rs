//! Lit mode's model on the CPU (decision 0023): the disc light's irradiance against a numerical
//! integral, the extracted key light keeping the light the camera sees, and the live
//! approximations conserving energy as the bake does.

use gs_render::matcap::{Options, Recipe, bake_under};
use gs_render::shading::cube::Cube;
use gs_render::shading::environment::SHIPPED;
use gs_render::shading::irradiance::irradiance;
use gs_render::shading::light::{cap_irradiance, extract, luminance};
use gs_render::shading::lit::{Lighting, LitEnvironment, shade};
use gs_render::shading::{Color, Environment, Material};
use std::f64::consts::PI;

/// Irradiance on a surface whose normal is at angle `t` from the center of a uniform cap of
/// radiance 1 and angular radius `s`, by the midpoint rule over the cap in its own frame.
fn cap_integral(t: f64, s: f64) -> f64 {
    let (rows, cols) = (400u32, 800u32);
    let n = [libm::sin(t), 0.0, libm::cos(t)];
    let mut sum = 0.0;
    for i in 0..rows {
        let theta = s * (f64::from(i) + 0.5) / f64::from(rows);
        let ring = libm::sin(theta) * (s / f64::from(rows)) * (2.0 * PI / f64::from(cols));
        for j in 0..cols {
            let phi = 2.0 * PI * (f64::from(j) + 0.5) / f64::from(cols);
            let d = [
                libm::sin(theta) * libm::cos(phi),
                libm::sin(theta) * libm::sin(phi),
                libm::cos(theta),
            ];
            sum += (n[0] * d[0] + n[2] * d[2]).max(0.0) * ring;
        }
    }
    sum
}

/// The cap's closed form (Snyder, as Lagarde and de Rousiers give it) is its integral, above,
/// across and below the horizon, for small, middling and large caps.
#[test]
fn a_disc_lights_a_surface_as_its_integral_does() {
    for s_deg in [2.0f64, 25.0, 60.0] {
        for t_deg in [0.0f64, 40.0, 80.0, 89.0, 95.0, 120.0] {
            let (t, s) = (t_deg.to_radians(), s_deg.to_radians());
            let want = cap_integral(t, s);
            let sin_s = libm::sin(s) as f32;
            let got = f64::from(cap_irradiance(libm::cos(t) as f32, sin_s * sin_s));
            let full = PI * libm::sin(s) * libm::sin(s);
            assert!(
                (got - want).abs() <= 2e-3 * full,
                "cap of {s_deg} degrees at {t_deg} degrees: {got} against {want}"
            );
        }
    }
}

/// Irradiance on the camera-facing card, summed over a cube's sharpest level.
fn card(cube: &Cube) -> [f32; 3] {
    irradiance(&cube.texels(0), [0.0, 0.0, 1.0])
}

/// Taking light 1 out of an environment keeps the light the camera sees: the rest plus the disc
/// give the camera-facing card the original's irradiance within 0.5%, in every shipped one.
#[test]
fn the_key_light_keeps_the_light_the_camera_sees() {
    for (name, _) in SHIPPED {
        let env = Environment::shipped(name).expect("a shipped environment builds");
        let (rest, key) = extract(&env.radiance).expect("every shipped one has a key");
        let before = luminance(card(&env.radiance));
        let rest_card = card(&rest);
        let after = luminance(rest_card) + luminance(key.irradiance([0.0, 0.0, 1.0]));
        assert!(
            (after - before).abs() <= 0.005 * before,
            "{name}: the card gets {after} with light 1 out, {before} before"
        );
        assert!(
            luminance(rest_card) < before,
            "{name}: light 1 holds some of the card's light"
        );
    }
}

/// White in a uniform white environment (the furnace): what one layer reflects the next one
/// down gets, so Lit mode returns exactly the light that falls, at every view angle.
#[test]
fn lit_mode_passes_the_furnace_test() {
    let env = LitEnvironment::of(&Environment::from_light("white", |_| [1.0; 3]));
    assert!(env.key.is_none(), "a uniform environment has no key light");
    let lit = Lighting {
        environment: &env,
        lights: &[],
        ambient: 1.0,
    };
    let looks = [
        Material {
            base_color: Color::WHITE,
            specular_roughness: 0.5,
            ..Material::default()
        },
        Material {
            base_color: Color::WHITE,
            specular_roughness: 0.2,
            coat_weight: 1.0,
            coat_roughness: 0.1,
            ..Material::default()
        },
    ];
    for m in looks {
        for mu in [1.0f32, 0.7, 0.3] {
            let v = [(1.0 - mu * mu).sqrt(), 0.0, mu];
            let out = shade(&m, &lit, [0.0, 0.0, 1.0], v, 1.0).total();
            for c in out {
                assert!(
                    (c - 1.0).abs() < 0.01,
                    "{m:?} seen at cosine {mu}: reflects {out:?}"
                );
            }
        }
    }
}

/// The luminance of Lit mode's diffuse and specular light and of the bake's, each summed over
/// the sphere's pixels (the rim left out, where both are sampled thinly).
fn sphere_sums(m: &Material, lit: &Lighting, baked: &gs_render::matcap::Baked) -> [f64; 4] {
    let size = baked.size;
    let half = size as f32 / 2.0;
    let mut sums = [0.0f64; 4];
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / half - 1.0;
            let w = (y as f32 + 0.5) / half - 1.0;
            if u * u + w * w > 0.9 {
                continue;
            }
            let n = [u, -w, (1.0 - u * u - w * w).sqrt()];
            let got = shade(m, lit, n, [0.0, 0.0, 1.0], 1.0);
            let (d, s) = baked.layers(x, y);
            let albedo = m.base_color.0;
            let diffuse: [f32; 3] = std::array::from_fn(|k| got.weight[k] * got.irradiance[k]);
            sums[0] += f64::from(luminance(diffuse));
            sums[1] += f64::from(luminance(std::array::from_fn(|k| albedo[k] * d[k])));
            sums[2] += f64::from(luminance(got.specular));
            sums[3] += f64::from(luminance(s));
        }
    }
    sums
}

/// Lit mode against the bake (the reference integrator) for the clay look under the default
/// studio, with the environment's rest baked on its own to tell the parts apart: the diffuse
/// (SH9 and the disc's cap irradiance) within 2%, light 1's specular (Karis's representative
/// point) within 5%, and the rest's split sum within 15% (it assumes the lobe seen head-on, so it
/// loses some light toward the rim; measured 10%).
#[test]
fn lit_mode_stays_near_the_bake() {
    let env = Environment::shipped("studio").expect("the default studio builds");
    let lit_env = LitEnvironment::shipped("studio").expect("the default studio prepares");
    let key: Vec<_> = lit_env.key.iter().copied().collect();
    let m = Material {
        base_color: Color([0.4, 0.26, 0.15]),
        specular_roughness: 0.5,
        ..Material::default()
    };
    let recipe = Recipe {
        name: "clay".into(),
        catalog_id: String::new(),
        environment: "studio".into(),
        material: m.clone(),
        bake: gs_render::matcap::Bake {
            size: 32,
            ..gs_render::matcap::Bake::default()
        },
        ..Recipe::default()
    };
    let lighting = |lights| Lighting {
        environment: &lit_env,
        lights,
        ambient: 1.0,
    };
    let whole = sphere_sums(
        &m,
        &lighting(&key),
        &bake_under(&recipe, &env, Options::default()),
    );
    let rest = sphere_sums(
        &m,
        &lighting(&[]),
        &bake_under(&recipe, &lit_env.rest, Options::default()),
    );
    let off = |got: f64, want: f64| got / want - 1.0;
    let diffuse = off(whole[0], whole[1]);
    let key_specular = off(whole[2] - rest[2], whole[3] - rest[3]);
    let split_sum = off(rest[2], rest[3]);
    eprintln!(
        "against the bake: diffuse {diffuse:+.4}, light 1's specular {key_specular:+.4}, the rest's split sum {split_sum:+.4}"
    );
    assert!(diffuse.abs() < 0.02, "the diffuse differs by {diffuse}");
    assert!(
        key_specular.abs() < 0.05,
        "light 1's specular differs by {key_specular}"
    );
    assert!(
        split_sum.abs() < 0.15,
        "the split sum differs by {split_sum}"
    );
}
