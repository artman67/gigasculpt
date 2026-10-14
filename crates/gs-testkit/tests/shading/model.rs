//! The BRDF obeys physics: the white furnace, energy conservation, reciprocity of the raw lobes.

use crate::{Brdf, narrow, view, wide};
use gs_render::shading::brdf::{Fresnel, alpha, ggx_lobe, lambert};
use gs_render::shading::{Brdf as _, Color, Material};
use gs_testkit::reference::shading::{Rgb, V, hemisphere, unit};

const N: V = [0.0, 0.0, 1.0];

/// Directional albedo: the share of light from everywhere that `brdf` reflects toward a viewer
/// at cosine `mu`, integrated by the slow reference.
fn albedo(m: &Material, mu: f64, brdf: Brdf) -> Rgb {
    let v = view(mu);
    let a = f64::from(alpha(m.specular_roughness));
    hemisphere(N, v, a, 96, &|l| {
        let f = brdf(m, narrow(N), narrow(v), narrow(l));
        wide(f).map(|x| x * l[2])
    })
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

/// A white, non-absorbing material under a uniform white environment reflects all of it, at
/// every roughness and view angle, metal and dielectric alike (Filament's furnace test). The
/// same metal with single scattering only loses light as it roughens, so the check bites.
#[test]
fn white_furnace() {
    for roughness in [0.2, 0.4, 0.6, 0.8, 1.0] {
        for mu in [0.1, 0.4, 0.7, 1.0] {
            for m in [white_metal(roughness), white_dielectric(roughness)] {
                let e = albedo(&m, mu, Material::brdf);
                assert!(
                    e.iter().all(|&c| (0.99..=1.003).contains(&c)),
                    "metalness {} roughness {roughness} at cos {mu}: albedo {e:?}",
                    m.base_metalness
                );
            }
        }
    }
    for mu in [0.1, 0.4, 0.7, 1.0] {
        let lost = albedo(&white_metal(1.0), mu, Material::single_scattering_brdf);
        assert!(
            lost[0] < 0.9,
            "single scattering loses light at roughness 1, cos {mu}: albedo {lost:?}"
        );
    }
}

/// No material reflects more light than reaches it: colored and tinted dielectrics, measured
/// metals with their F82 tints, half metals, coats, at every roughness and view angle.
#[test]
fn energy_never_exceeds_input() {
    let chromium = Material {
        base_color: Color([0.654, 0.685, 0.701]),
        specular_color: Color([0.688, 0.728, 0.798]),
        base_metalness: 1.0,
        ..Material::default()
    };
    let materials = [
        white_dielectric(0.5),
        Material {
            base_color: Color([0.9, 0.2, 0.05]),
            specular_ior: 2.5,
            specular_color: Color([1.0, 0.6, 0.3]),
            ..Material::default()
        },
        chromium.clone(),
        Material {
            base_metalness: 0.5,
            ..chromium
        },
        Material {
            coat_weight: 1.0,
            coat_roughness: 0.1,
            ..white_dielectric(0.7)
        },
        Material {
            coat_weight: 0.6,
            coat_color: Color([0.8, 0.9, 1.0]),
            ..white_metal(0.4)
        },
    ];
    for m in &materials {
        for roughness in [0.2, 0.5, 1.0] {
            let m = Material {
                specular_roughness: roughness,
                ..m.clone()
            };
            for mu in [0.05, 0.3, 0.7, 1.0] {
                let e = albedo(&m, mu, Material::brdf);
                assert!(
                    e.iter().all(|&c| c <= 1.003),
                    "{m:?} at cos {mu}: albedo {e:?}"
                );
            }
        }
    }
}

/// The raw lobes give the same value with light and viewer swapped (Helmholtz reciprocity).
/// The compensated, layered BRDF does not, by design: `OpenPBR`'s albedo scaling looks only at
/// the viewer's side.
#[test]
fn raw_lobes_are_reciprocal() {
    let f = Fresnel::f82([0.7, 0.5, 0.3], [1.0; 3], [0.8, 0.9, 1.0]);
    let directions = [
        [0.3, 0.2, 0.9],
        [-0.6, 0.1, 0.5],
        [0.1, -0.8, 0.3],
        [0.9, 0.05, 0.15],
        [-0.2, -0.3, 0.95],
    ]
    .map(|d| narrow(unit(d)));
    for roughness in [0.1, 0.5, 1.0] {
        for v in directions {
            for l in directions {
                let a = alpha(roughness);
                let (there, back) = (
                    ggx_lobe(&f, a, narrow(N), v, l),
                    ggx_lobe(&f, a, narrow(N), l, v),
                );
                for k in 0..3 {
                    let scale = there[k].abs().max(1e-3);
                    assert!(
                        (there[k] - back[k]).abs() <= 1e-4 * scale,
                        "GGX at roughness {roughness}, {v:?} and {l:?}: {there:?} against {back:?}"
                    );
                }
                assert_eq!(
                    lambert([0.5; 3], narrow(N), v, l),
                    lambert([0.5; 3], narrow(N), l, v),
                    "Lambert"
                );
            }
        }
    }
}
