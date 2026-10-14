//! Lit mode on the GPU (decision 0023) against its CPU twin (`shading::lit`), and the light
//! fixed to the camera.

use crate::common::{diff, front_ortho, lavapipe, shipped, snap, upload};
use crate::scene;
use gs_render::display::encode;
use gs_render::shading::light::Lights;
use gs_render::shading::lit::{Lighting, LitEnvironment, shade};
use gs_render::shading::{Color, Material};
use gs_render::{
    Camera, Display, Fit, Image, Lens, Light, Occlusion, Preset, Projection, Shading, View,
    ViewTransform,
};

pub const SIZE: u32 = 129;
/// The default display without dither.
const PLAIN: Display = Display {
    exposure: 0.0,
    transform: ViewTransform::PbrNeutral,
    dither: false,
};

/// The materials checked: the clay look, a sharp metal, and a coated red.
fn materials() -> Vec<(&'static str, Material)> {
    let coated = Material {
        base_color: Color([0.5, 0.05, 0.03]),
        specular_roughness: 0.4,
        coat_weight: 1.0,
        coat_roughness: 0.1,
        ..Material::default()
    };
    vec![
        ("clay", shipped("light-brown-clay").material),
        ("metal", shipped("metal-1").material),
        ("coated", coated),
    ]
}

/// A camera-facing unit sphere in Lit mode, occlusion off.
fn gpu_picture(m: &Material, lights: &Lights, display: Display) -> Image {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(5));
    let view = View {
        display,
        occlusion: Occlusion::OFF,
        light: Light {
            environment: None,
            lights,
        },
        ..front_ortho(Shading::Lit(m))
    };
    snap(r, &mesh, &view, SIZE)
}

/// Pixel `(x, y)`'s point on the sphere seen by `front_ortho`, or `None` off it and near its
/// rim (where the mesh's outline and the true circle part).
pub fn on_sphere(x: u32, y: u32) -> Option<[f32; 3]> {
    let at = |i: u32| ((i as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0) * 1.1;
    let (u, w) = (at(x), -at(y));
    let rho = u * u + w * w;
    (rho <= 0.9).then(|| [u, w, (1.0 - rho).sqrt()])
}

/// The mean and largest difference in 8-bit levels, over the sphere's pixels, between the
/// GPU's picture and the CPU evaluator's light for `lit`, shown as the display pass shows it.
fn against_cpu(gpu: &Image, m: &Material, lit: &Lighting) -> (f64, u8) {
    let (mut sum, mut count, mut worst) = (0u64, 0u64, 0u8);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let Some(n) = on_sphere(x, y) else { continue };
            let light = shade(m, lit, n, [0.0, 0.0, 1.0], 1.0).total();
            let want = encode(PLAIN.show(light));
            let got = gpu.pixel(x, y);
            for k in 0..3 {
                let d = got[k].abs_diff(want[k]);
                sum += u64::from(d);
                worst = worst.max(d);
                count += 1;
            }
        }
    }
    (sum as f64 / count as f64, worst)
}

/// Every light off.
fn dark() -> Lights {
    let mut lights = Lights::DEFAULT;
    lights.each[0].on = false;
    lights
}

/// The GPU's Lit branch is its CPU twin: a camera-facing sphere of each material, under the
/// default studio with its key light and with every light off (only the rest of the
/// environment), within 1 sRGB level on average over the sphere.
#[test]
fn lit_mode_on_the_gpu_is_its_cpu_twin() {
    let env = LitEnvironment::shipped("studio").expect("the default studio prepares");
    let key: Vec<_> = env.key.iter().copied().collect();
    for (name, m) in materials() {
        for (lights, discs, label) in [
            (Lights::DEFAULT, &key[..], "key light on"),
            (dark(), &[][..], "lights off"),
        ] {
            let lit = Lighting {
                environment: &env,
                lights: discs,
                ambient: 1.0,
            };
            let picture = gpu_picture(&m, &lights, PLAIN);
            let (mean, worst) = against_cpu(&picture, &m, &lit);
            eprintln!("{name}, {label}: {mean:.3} levels on average, {worst} at most");
            assert!(
                mean <= 1.0,
                "{name}, {label}: the GPU is {mean} levels from the CPU on average"
            );
        }
    }
}

/// Lights only add light: turning light 1 on brightens or keeps every channel of every pixel.
/// Checked under the Standard transform, 2 stops down: PBR Neutral's shoulder pulls the other
/// channels down as one grows, so its picture of more light can be darker in places.
#[test]
fn light_1_only_adds_light() {
    let display = Display {
        exposure: -2.0,
        transform: ViewTransform::Standard,
        dither: false,
    };
    for (name, m) in materials() {
        let on = gpu_picture(&m, &Lights::DEFAULT, display);
        let off = gpu_picture(&m, &dark(), display);
        let darker = (on.rgba.iter().zip(&off.rgba))
            .filter(|(a, b)| a < b)
            .count();
        assert_eq!(
            darker, 0,
            "{name}: {darker} channels darken with light 1 on"
        );
        let (max, _) = diff(&on, &off);
        assert!(max > 20, "{name}: light 1 lights the sphere (max {max})");
    }
}

/// The lit side follows light 1 (catalog items `material.standard-shading` and `light.palette`):
/// lit by light 1 alone (Ambient 0), from the left and then from the right, the half of the
/// sphere facing the light is the brighter one, and the two pictures mirror each other.
#[test]
fn the_lit_side_follows_light_1() {
    let m = shipped("light-brown-clay").material;
    let from = |x: f32| {
        let mut lights = Lights {
            ambient: 0.0,
            ..Lights::DEFAULT
        };
        lights.each[0].direction = Some([x, 0.2, 0.6]);
        gpu_picture(&m, &lights, PLAIN)
    };
    let halves = |image: &Image| {
        let (mut left, mut right) = (0u64, 0u64);
        for y in 0..SIZE {
            for x in 0..SIZE {
                if on_sphere(x, y).is_none() {
                    continue;
                }
                let p = image.pixel(x, y);
                let sum = u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]);
                if x < SIZE / 2 {
                    left += sum;
                } else if x > SIZE / 2 {
                    right += sum;
                }
            }
        }
        (left, right)
    };
    let (left, right) = (from(-0.8), from(0.8));
    let (a, b) = (halves(&left), halves(&right));
    assert!(
        a.0 > 2 * a.1,
        "lit from the left, the left half is brighter: {a:?}"
    );
    assert!(
        b.1 > 2 * b.0,
        "lit from the right, the right half is brighter: {b:?}"
    );
    let mirrored = |x: u64, y: u64| x.abs_diff(y) * 100 <= x.max(y);
    assert!(
        mirrored(a.0, b.1) && mirrored(a.1, b.0),
        "the two pictures mirror each other within 1%: {a:?} {b:?}"
    );
}

/// The environment and the lights are fixed to the camera: a quad sphere (which a quarter turn
/// maps onto itself) seen from the front and from the right after a quarter orbit gives the same
/// picture, within 2 levels where the two views split a quad along different diagonals. Lit
/// from the world instead, the key light would swing to the other side.
#[test]
fn the_light_turns_with_the_camera() {
    let r = lavapipe();
    let mesh = upload(r, &scene::quad_sphere(24));
    let m = shipped("light-brown-clay").material;
    let at = |preset| {
        let camera = Camera {
            distance: 1.1,
            lens: Lens::from_fov(std::f32::consts::FRAC_PI_2, Fit::Vertical),
            projection: Projection::Orthographic,
            ..Camera::preset(preset)
        };
        let view = View {
            occlusion: Occlusion::OFF,
            ..View::new(camera, Shading::Lit(&m))
        };
        snap(r, &mesh, &view, 96)
    };
    let (max, count) = diff(&at(Preset::Front), &at(Preset::Right));
    assert!(
        max <= 2,
        "the same picture after a quarter orbit (max {max}, {count} pixels differ)"
    );
}
