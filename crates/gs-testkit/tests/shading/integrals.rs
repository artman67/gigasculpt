//! gs-render's precomputed integrals against direct integration in f64: the DFG table, the
//! irradiance map, the SH9 fit and GGX-prefiltered radiance.

use crate::{narrow, wide};
use gs_render::shading::Environment;
use gs_render::shading::brdf::alpha;
use gs_render::shading::dfg::Dfg;
use gs_render::shading::lobe::prefiltered;
use gs_testkit::reference::shading::{self as reference, Rgb, V, dot, unit};

/// Each of the table's three integrals, at and between its entries, within 0.01 of direct
/// integration.
#[test]
fn dfg_table_matches_direct_integration() {
    let table = Dfg::get();
    for roughness in [0.05f32, 0.27, 0.5, 0.81, 1.0] {
        for mu in [0.02f32, 0.13, 0.5, 0.77, 1.0] {
            let got = table.at(mu, roughness);
            let want = reference::dfg(f64::from(mu), f64::from(alpha(roughness)), 128);
            let pairs = [
                (got.total, want[0]),
                (got.schlick, want[1]),
                (got.dip, want[2]),
            ];
            for (k, (g, w)) in pairs.into_iter().enumerate() {
                assert!(
                    (f64::from(g) - w).abs() < 0.01,
                    "integral {k} at cos {mu}, roughness {roughness}: {g} against {w}"
                );
            }
        }
    }
}

/// A studio-like light: a gray room brighter above, and a bright disc of light at upper left.
fn studio_like(d: V) -> Rgb {
    let key = unit([-0.45, 0.55, 0.7]);
    let room = 0.2 + 0.1 * d[1];
    let lamp = if dot(d, key) > 0.95 { 30.0 } else { 0.0 };
    [room + lamp, room + 0.8 * lamp, room + 0.6 * lamp]
}

/// A smooth sky: nothing sharper than a broad glow.
fn sky(d: V) -> Rgb {
    let sun = unit([0.3, 0.8, 0.5]);
    let glow = libm::pow(dot(d, sun).max(0.0), 4.0);
    [0.3 + 2.0 * glow, 0.35 + 1.8 * glow, 0.5 + 1.2 * glow]
}

fn normals() -> Vec<V> {
    let mut out = vec![
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, -1.0],
    ];
    out.extend(
        [
            [-0.5, 0.6, 0.6],
            [0.7, -0.2, 0.6],
            [1.0, 0.0, 0.0],
            [-0.3, -0.9, -0.2],
        ]
        .map(unit),
    );
    out
}

/// What a metered environment does to `light`: scales it so a white diffuser facing the
/// camera reflects 1.
fn metered(directions: &[(V, f64)], light: fn(V) -> Rgb) -> impl Fn(V) -> Rgb {
    let e = reference::irradiance(directions, &light, [0.0, 0.0, 1.0]);
    let k = std::f64::consts::PI / (0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2]);
    move |d| light(d).map(|c| c * k)
}

fn worst(pairs: impl Iterator<Item = (Rgb, Rgb)>) -> (f64, f64) {
    let (mut off, mut top) = (0.0f64, 0.0f64);
    for (a, b) in pairs {
        for k in 0..3 {
            off = off.max((a[k] - b[k]).abs());
            top = top.max(b[k]);
        }
    }
    (off, top)
}

/// The irradiance map is the direct integral within 1.5% of the brightest value; the SH9 fit
/// is the same fit the reference computes, within 1%, and for a smooth sky it stays within 3%
/// of the true irradiance (Ramamoorthi and Hanrahan report about 1% on average).
#[test]
fn irradiance_and_sh9_match_direct_integration() {
    let directions = reference::sphere(256);
    for (name, light) in [("studio", studio_like as fn(V) -> Rgb), ("sky", sky)] {
        let env = Environment::from_light(name, |d| narrow(light(wide(d))));
        let truth = metered(&directions, light);
        let map = normals().into_iter().map(|n| {
            (
                wide(env.irradiance(narrow(n))),
                reference::irradiance(&directions, &truth, n),
            )
        });
        let (off, top) = worst(map);
        assert!(
            off < 0.015 * top,
            "{name} irradiance map: off by {off} of {top}"
        );
        let fit = normals().into_iter().map(|n| {
            (
                wide(env.sh9.irradiance(narrow(n))),
                reference::sh9_irradiance(&directions, &truth, n),
            )
        });
        let (off, top) = worst(fit);
        assert!(off < 0.01 * top, "{name} SH9: off by {off} of {top}");
    }
    let env = Environment::from_light("sky", |d| narrow(sky(wide(d))));
    let truth = metered(&directions, sky);
    let fit = normals().into_iter().map(|n| {
        (
            wide(env.sh9.irradiance(narrow(n))),
            reference::irradiance(&directions, &truth, n),
        )
    });
    let (off, top) = worst(fit);
    assert!(
        off < 0.03 * top,
        "SH9 against the true sky irradiance: off by {off} of {top}"
    );
}

/// GGX-prefiltered radiance by filtered importance sampling (256 samples, as a one-time
/// prefilter can afford) is the lobe's mean radiance
/// within 3%, at three roughnesses and in several directions.
#[test]
fn prefiltered_radiance_matches_direct_integration() {
    let directions = reference::sphere(128);
    let env = Environment::from_light("sky", |d| narrow(sky(wide(d))));
    let truth = metered(&directions, sky);
    for roughness in [0.3f32, 0.6, 1.0] {
        let a = f64::from(alpha(roughness));
        for r in normals() {
            let got = wide(prefiltered(&env, narrow(r), roughness, 256));
            let want = reference::prefiltered(&truth, r, a, 128);
            for k in 0..3 {
                assert!(
                    (got[k] - want[k]).abs() <= 0.03 * want[k],
                    "roughness {roughness} along {r:?}: {got:?} against {want:?}"
                );
            }
        }
    }
}

/// A Radiance file reads back: header, a run-length scanline (runs and literals) and a flat
/// one, each pixel `mantissa * 2^(exponent - 136)`. A file that is not RGBE is refused.
#[test]
fn hdr_files_read() {
    use gs_render::shading::hdr::Equirect;
    let mut file = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 2 +X 8\n".to_vec();
    // Row 0, run-length (2, 2, width): red a run of 8 times 128 (136 = run of 8), green 8
    // literals 0..8, blue a run of 64, the exponent a run of 129.
    file.extend([
        2, 2, 0, 8, 136, 128, 8, 0, 1, 2, 3, 4, 5, 6, 7, 136, 64, 136, 129,
    ]);
    // Row 1, flat: every pixel (16, 32, 64, 130).
    for _ in 0..8 {
        file.extend([16, 32, 64, 130]);
    }
    let image = Equirect::read_hdr(&file).expect("reads");
    assert_eq!((image.width, image.height), (8, 2));
    let bits = |p: [f32; 3]| p.map(f32::to_bits);
    for (x, p) in image.pixels[..8].iter().enumerate() {
        let want = [1.0, x as f32 / 128.0, 0.5];
        assert_eq!(bits(*p), bits(want), "row 0 pixel {x}: {p:?}");
    }
    let row1 = bits([0.25, 0.5, 1.0]);
    assert!(
        image.pixels[8..].iter().all(|p| bits(*p) == row1),
        "row 1: {:?}",
        &image.pixels[8..]
    );
    let refused = Equirect::read_hdr(b"#?RADIANCE\nFORMAT=32-bit_rle_xyze\n\n-Y 1 +X 1\n\0\0\0\0");
    assert!(refused.is_err(), "XYZ files are refused");
}
