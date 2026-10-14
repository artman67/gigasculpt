//! The MatCap baker: a look's material on a ball under an environment (research 11, 12.3); and
//! Flat Color, the material that is never baked.

use crate::common::{front_ortho, lavapipe, shipped, snap, upload};
use crate::scene;
use gs_protocol::{AttrDomain, AttrValues, Attribute};
use gs_render::display::{decode, encode};
use gs_render::matcap::{Bake, Options, Recipe, bake_under};
use gs_render::shading::light::Lights;
use gs_render::shading::lit::LitEnvironment;
use gs_render::shading::{Color, Environment, Material};
use gs_render::{Display, Light, Shading, View, ViewTransform};

/// A look of `material` baked `size` pixels wide.
fn look(material: Material, size: u32) -> Recipe {
    Recipe {
        name: "test".into(),
        material,
        bake: Bake {
            size,
            ..Bake::default()
        },
        ..Recipe::default()
    }
}

fn white_metal(roughness: f32) -> Material {
    Material {
        base_color: Color::WHITE,
        base_metalness: 1.0,
        specular_roughness: roughness,
        ..Material::default()
    }
}

fn white_dielectric(roughness: f32) -> Material {
    Material {
        base_color: Color::WHITE,
        specular_roughness: roughness,
        ..Material::default()
    }
}

/// The white furnace on the baked MatCap itself (catalog item `material.look-recipe`): under a
/// uniform white environment, a white metal, a white dielectric, a white scattering dielectric
/// and a white coated one show 99 to 100% of the light at every pixel, at roughness 0, 0.5
/// and 1. Baked without energy compensation, the rough metal loses light.
#[test]
fn white_looks_vanish_in_white_light() {
    let white = Environment::from_light("white", |_| [1.0; 3]);
    let mut looks = Vec::new();
    for roughness in [0.0, 0.5, 1.0] {
        looks.push(white_metal(roughness));
        looks.push(white_dielectric(roughness));
        looks.push(Material {
            subsurface_weight: 1.0,
            subsurface_radius: 8.0,
            ..white_dielectric(roughness)
        });
        looks.push(Material {
            coat_weight: 1.0,
            coat_roughness: 0.1,
            ..white_dielectric(roughness)
        });
    }
    for m in looks {
        let baked = bake_under(&look(m.clone(), 24), &white, Options::default());
        for (x, y) in (0..24).flat_map(|y| (0..24).map(move |x| (x, y))) {
            let shown = baked.radiance(x, y);
            assert!(
                shown.iter().all(|&c| (0.99..=1.002).contains(&c)),
                "{m:?} pixel ({x}, {y}) shows {shown:?}"
            );
        }
    }
    let off = Options {
        energy_compensation: false,
    };
    let baked = bake_under(&look(white_metal(1.0), 24), &white, off);
    let center = baked.radiance(12, 12);
    assert!(
        center[0] < 0.9,
        "single scattering loses light at roughness 1: {center:?}"
    );
}

/// A perfect mirror shows the environment: a white metal at roughness 0 reflects, at every
/// pixel of the ball, the light along the mirror direction of its normal (within 1%, the
/// half-float layers and the 2 by 2 pixel samples).
#[test]
fn a_mirror_reflects_the_environment() {
    let lamp = |d: [f32; 3]| d[0] < -0.3 && d[1] > 0.4;
    let env = Environment::from_light("lamp", |d| {
        let on = if lamp(d) { 6.0 } else { 0.0 };
        [0.2 + 0.1 * d[1] + on, 0.3 + on, 0.4 - 0.1 * d[0] + on]
    });
    let size = 32;
    let baked = bake_under(&look(white_metal(0.0), size), &env, Options::default());
    let mut checked = 0;
    for (x, y) in (0..size).flat_map(|y| (0..size).map(move |x| (x, y))) {
        let (u, v) = ((x as f32 + 0.5) / 16.0 - 1.0, (y as f32 + 0.5) / 16.0 - 1.0);
        // Away from the rim and from the lamp's edges, where the 2 by 2 pixel samples differ.
        let z2 = 1.0 - u * u - v * v;
        if z2 < 0.2 {
            continue;
        }
        let n = [u, -v, z2.sqrt()];
        let r = [
            2.0 * n[2] * n[0],
            2.0 * n[2] * n[1],
            2.0 * n[2] * n[2] - 1.0,
        ];
        let want = env.radiance(r, 0.0);
        let steps = [[0.05, 0.0], [-0.05, 0.0], [0.0, 0.05], [0.0, -0.05]];
        if steps
            .iter()
            .any(|s| lamp([r[0] + s[0], r[1] + s[1], r[2]]) != lamp(r))
        {
            continue;
        }
        let got = baked.layers(x, y).1;
        for k in 0..3 {
            assert!(
                (got[k] - want[k]).abs() <= 0.01 * want[k].max(0.1),
                "pixel ({x}, {y}): {got:?} against {want:?}"
            );
        }
        checked += 1;
    }
    assert!(checked > 300, "most of the ball was checked ({checked})");
}

/// A bake is the same bytes on 1 thread and on 8, for a scattering look and a sharp metal.
#[test]
fn bakes_are_deterministic_at_any_thread_count() {
    let env = Environment::shipped("studio").expect("the shipped studio builds");
    for name in ["red-wax", "chrome-a"] {
        let small = Recipe {
            bake: Bake {
                size: 48,
                ..Bake::default()
            },
            ..shipped(name)
        };
        let one = gs_par::with_threads(1, || bake_under(&small, &env, Options::default()));
        let eight = gs_par::with_threads(8, || bake_under(&small, &env, Options::default()));
        assert!(
            one == eight,
            "{name} bakes the same bytes on 1 and 8 threads"
        );
    }
}

/// Flat Color (decision 0043, step (c)) is never baked: each pixel is the paint times the base
/// color, shown as stored. On a sphere painted red where x is above 0.2, the painted side shows
/// paint x base and the rest the base, within a level, under the default studio with its key
/// light, occlusion and dither on; with every light off, a captured environment, exposure +2 and
/// `AgX` the picture is byte for byte the same.
#[test]
fn flat_color_is_paint_times_base_under_any_light() {
    const SIZE: u32 = 96;
    let r = lavapipe();
    let red = [200u8, 45, 45];
    let mut sphere = scene::icosphere(4);
    let words = (sphere.positions.iter())
        .map(|p| {
            let [r, g, b] = if p[0] > 0.2 { red } else { [255; 3] };
            u32::from_le_bytes([r, g, b, 255])
        })
        .collect();
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::U32(words),
    };
    sphere.attributes.insert("color".into(), column);
    let mesh = upload(r, &sphere);
    let base = [0.5, 1.0, 0.25];
    let studio = front_ortho(Shading::Flat(base));
    let mut dark = Lights::DEFAULT;
    dark.each[0].on = false;
    let env = LitEnvironment::shipped("photo-studio-01").expect("the captured studio prepares");
    let env = r.upload_environment(&env);
    let other = View {
        display: Display {
            exposure: 2.0,
            transform: ViewTransform::Agx,
            dither: false,
        },
        light: Light {
            environment: Some(&env),
            lights: &dark,
        },
        ..studio
    };
    let picture = snap(r, &mesh, &studio, SIZE);
    assert!(
        picture == snap(r, &mesh, &other, SIZE),
        "the light, the environment and the tone change no pixel"
    );
    let shown = |srgb: [u8; 3]| {
        let paint = decode(srgb);
        encode(std::array::from_fn(|k| paint[k] * base[k]))
    };
    let (mut painted, mut plain) = (0, 0);
    let scale = 2.2 / SIZE as f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (wx, wy) = (
                (x as f32 + 0.5) * scale - 1.1,
                (y as f32 + 0.5) * scale - 1.1,
            );
            let want = match wx {
                _ if wx * wx + wy * wy > 0.8 => continue,
                wx if wx > 0.35 => shown(red),
                wx if wx < 0.05 => shown([255; 3]),
                _ => continue,
            };
            let p = picture.pixel(x, y);
            let off = (0..3).map(|k| p[k].abs_diff(want[k])).max();
            assert!(off <= Some(1), "({x}, {y}) shows {p:?}, not {want:?}");
            if want == shown(red) {
                painted += 1;
            } else {
                plain += 1;
            }
        }
    }
    assert!(
        painted > 300 && plain > 300,
        "both sides checked: {painted}, {plain}"
    );
}
