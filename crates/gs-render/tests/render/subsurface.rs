//! Live subsurface (decision 0023, `material.wax-preview`): a radius or Strength of 0 changes nothing,
//! scattering moves diffuse light without making or losing it, and skin carries red farther
//! than blue.

use crate::common::{front_ortho, lavapipe, shipped, upload};
use crate::lit::{SIZE, on_sphere};
use crate::scene;
use gs_render::display::decode;
use gs_render::shading::Material;
use gs_render::shading::light::Lights;
use gs_render::shading::lit::LitEnvironment;
use gs_render::{
    Display, Image, Light, RenderSize, Shading, Subsurface, SubsurfaceMode, View, ViewTransform,
};

/// Linear light, 1 stop down so nothing clips, without dither.
const LINEAR: Display = Display {
    exposure: -1.0,
    transform: ViewTransform::Standard,
    dither: false,
};

fn subsurface(mode: SubsurfaceMode, radius: f32) -> Subsurface {
    Subsurface {
        mode,
        radius,
        ..Subsurface::DEFAULT
    }
}

/// Skin 4 without its reflections, so every pixel is diffuse light.
fn diffuse_skin() -> Material {
    Material {
        specular_weight: 0.0,
        ..shipped("skin-4").material
    }
}

/// A camera-facing unit sphere of `m` in Lit mode with `lights` and subsurface `s`, averaged
/// over 8 samples.
fn picture(m: &Material, lights: &Lights, s: Subsurface) -> Image {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(5));
    let view = View {
        display: LINEAR,
        light: Light {
            environment: None,
            lights,
        },
        subsurface: s,
        ..front_ortho(Shading::Lit(m))
    };
    let size = RenderSize {
        samples: 8,
        ..RenderSize::square(SIZE)
    };
    r.snapshot(&mesh, &view, size).expect("renders")
}

fn linear(image: &Image, x: u32, y: u32) -> [f32; 3] {
    let p = image.pixel(x, y);
    decode([p[0], p[1], p[2]])
}

/// Radius 0, a radius whose farthest sample stays inside one pixel, or Strength 0 leaves the
/// picture exactly as subsurface off draws it.
#[test]
fn a_radius_of_0_changes_nothing() {
    let m = shipped("skin-4").material;
    let lights = Lights::DEFAULT;
    let off = picture(&m, &lights, subsurface(SubsurfaceMode::Off, 1.0));
    for radius in [0.0, 1e-3] {
        let on = picture(&m, &lights, subsurface(SubsurfaceMode::On, radius));
        assert!(on.rgba == off.rgba, "radius {radius} changes the picture");
    }
    let none = Subsurface {
        strength: 0.0,
        ..subsurface(SubsurfaceMode::On, 1.0)
    };
    assert!(
        picture(&m, &lights, none).rgba == off.rgba,
        "Strength 0 changes the picture"
    );
}

/// Scattering moves diffuse light under the surface but neither makes nor loses it: the
/// diffuse light summed over the sphere in linear light stays within 1% per channel.
#[test]
fn scattering_keeps_the_diffuse_light() {
    let m = diffuse_skin();
    let lights = Lights::DEFAULT;
    let off = picture(&m, &lights, subsurface(SubsurfaceMode::Off, 1.0));
    let on = picture(&m, &lights, subsurface(SubsurfaceMode::On, 1.0));
    let background = linear(&off, 0, 0);
    let total = |image: &Image| {
        let mut sum = [0.0f64; 3];
        for y in 0..SIZE {
            for x in 0..SIZE {
                let c = linear(image, x, y);
                for k in 0..3 {
                    sum[k] += f64::from(c[k] - background[k]);
                }
            }
        }
        sum
    };
    let peak = (off.rgba.chunks(4).flat_map(|p| &p[..3]))
        .copied()
        .max()
        .unwrap_or(0);
    assert!(peak < 255, "nothing clips");
    let (with, without) = (total(&on), total(&off));
    let (max, _) = crate::common::diff(&on, &off);
    assert!(max > 4, "skin scatters visibly (max {max} levels)");
    for k in 0..3 {
        let ratio = with[k] / without[k];
        eprintln!("channel {k}: {ratio:.5} of the light without scattering");
        assert!(
            (ratio - 1.0).abs() <= 0.01,
            "channel {k}: scattering changes the diffuse light by {:.2}%",
            (ratio - 1.0) * 100.0
        );
    }
}

/// Skin carries red farther than blue: lit by light 1 alone (Ambient 0), the light scattering
/// adds on the sphere's dark side, relative to the lit side, is larger in red than in blue.
#[test]
fn skin_carries_red_farther_than_blue() {
    let env = LitEnvironment::shipped("studio").expect("the default studio prepares");
    let key = env.key.expect("the studio has a key light").direction;
    let m = diffuse_skin();
    let lights = Lights {
        ambient: 0.0,
        ..Lights::DEFAULT
    };
    let off = picture(&m, &lights, subsurface(SubsurfaceMode::Off, 1.0));
    let on = picture(&m, &lights, subsurface(SubsurfaceMode::On, 1.0));
    let (mut dark, mut lit) = ([0.0f64; 3], [0.0f64; 3]);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let Some(n) = on_sphere(x, y) else { continue };
            let facing = n[0] * key[0] + n[1] * key[1] + n[2] * key[2];
            let (a, b) = (linear(&on, x, y), linear(&off, x, y));
            for k in 0..3 {
                if facing < 0.0 {
                    dark[k] += f64::from(a[k] - b[k]);
                } else {
                    lit[k] += f64::from(b[k]);
                }
            }
        }
    }
    let carried: Vec<f64> = (0..3).map(|k| dark[k] / lit[k]).collect();
    eprintln!("carried past the terminator, per channel: {carried:?}");
    assert!(
        carried[0] > 0.0 && carried[0] > carried[2],
        "red reaches farther than blue: {carried:?}"
    );
}
