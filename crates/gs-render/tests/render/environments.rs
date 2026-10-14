//! Environments (decision 0019): procedural studios and captured probes are one model. Every
//! shipped one is metered the same way, the probes' measured turn puts their dominant light on
//! the side of the default studio's key light, and a file is one kind or the other. A user's
//! file at the edges of RGBE's range still lights the subject or is refused with a reason.

use crate::common::{diff, front_ortho, lavapipe, luma, snap, upload};
use crate::scene;
use gs_render::shading::Environment;
use gs_render::shading::environment::{SHIPPED, Source};
use gs_render::shading::hdr::Equirect;
use gs_render::shading::light::Lights;
use gs_render::shading::lit::LitEnvironment;
use gs_render::{Light, Occlusion, Shading, View, clay_material};

const PI: f32 = std::f32::consts::PI;

fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// The azimuth (from +z toward +x) of an environment's dominant light: the first band of its
/// spherical-harmonic fit, which holds y, z and x.
fn dominant_azimuth(env: &Environment) -> f32 {
    let c = env.sh9.coefficients;
    libm::atan2f(luminance(c[3]), luminance(c[2]))
}

#[test]
fn every_environment_is_metered_alike() {
    for (name, _) in SHIPPED {
        let env = Environment::shipped(name).expect("a shipped environment builds");
        // A white diffuser facing the camera reflects irradiance over pi.
        let facing = luminance(env.irradiance([0.0, 0.0, 1.0])) / PI;
        assert!(
            (facing - 1.0).abs() < 0.02,
            "{name}: a white diffuser facing the camera reflects {facing}, not 1"
        );
    }
}

#[test]
fn probes_turn_their_dominant_light_to_the_key_side() {
    let key = libm::atan2f(-0.45, 0.70);
    let probes = SHIPPED
        .iter()
        .filter(|(_, text)| matches!(Source::parse(text), Ok(Source::Probe { .. })));
    let mut count = 0;
    for (name, _) in probes {
        let env = Environment::shipped(name).expect("a shipped probe builds");
        let turn = dominant_azimuth(&env) - key;
        let off = libm::atan2f(libm::sinf(turn), libm::cosf(turn)).abs();
        assert!(
            off < 0.035,
            "{name}: the dominant light is {off} radians from the key's azimuth"
        );
        count += 1;
    }
    assert!(count >= 3, "the three studio probes are shipped");
}

#[test]
fn an_environment_is_a_studio_or_a_probe() {
    let both = "name = \"both\"\n[room]\nwalls = 1.0\nceiling = 1.0\nfloor_albedo = 0.2\n\
                [probe]\nimage = \"x.hdr\"\nsource = \"here\"\n";
    let neither = "name = \"neither\"\n";
    for text in [both, neither] {
        assert!(Source::parse(text).is_err(), "refused: {text}");
    }
    let missing = Source::parse("name = \"p\"\n[probe]\nimage = \"nowhere.hdr\"\nsource = \"x\"\n")
        .expect("a probe file reads");
    let built = Environment::from_source(&missing, |_| None);
    assert!(
        built.is_err_and(|e| e.to_string().contains("nowhere.hdr")),
        "a probe without its image is refused, naming it"
    );
}

/// Linear 1.0 in RGBE.
const ONE: [u8; 4] = [128, 128, 128, 129];
/// 255 * 2^56, just under the brightest a file may hold, 2^64.
const UNDER: [u8; 4] = [255, 255, 255, 192];

/// A 64 by 32 Radiance file, flat: every pixel `background` but the `(x, y, rgbe)` listed. The
/// center column faces the camera, the edge columns are behind the subject.
fn radiance_file(background: [u8; 4], lit: &[(usize, usize, [u8; 4])]) -> Vec<u8> {
    let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 32 +X 64\n".to_vec();
    for y in 0..32 {
        for x in 0..64 {
            let found = lit.iter().find(|l| (l.0, l.1) == (x, y));
            bytes.extend(found.map_or(background, |l| l.2));
        }
    }
    bytes
}

/// Review 0005, m4: RGBE reaches 1.7e38, and such a texel left the picture black without a
/// word. A file is refused above 2^64 (`environment::BRIGHTEST`), naming the pixel.
#[test]
fn a_file_brighter_than_any_light_is_refused() {
    let read = |texel| Equirect::read_hdr(&radiance_file(ONE, &[(32, 16, texel)]));
    assert!(read(UNDER).is_ok(), "just under 2^64 reads");
    for texel in [[255, 255, 255, 193], [255, 255, 255, 255]] {
        let refused = read(texel).expect_err("brighter than 2^64").to_string();
        assert!(
            refused.contains("pixel 32 of row 16") && refused.contains("brighter than any light"),
            "{refused}"
        );
    }
}

/// Review 0005, m4: the brightest file allowed still lights a Lit sphere, with no black pixel.
/// A light behind the subject is not taken out as light 1, so its texels reach the GPU, which
/// holds them at the largest half float; a camera side near RGBE's darkest needs a metering
/// scale past f32's range, and metering holds what it lifts at 2^64.
#[test]
fn an_extreme_file_still_lights_the_subject() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(5));
    let picture = |bytes: &[u8]| {
        let image = Equirect::read_hdr(bytes).expect("allowed");
        let lit = LitEnvironment::of(&image.environment("extreme", 0.0));
        let cubes = [&lit.rest.radiance, &lit.rest.irradiance, &lit.prefiltered];
        let texels = cubes.into_iter().flat_map(|c| c.levels.iter().flatten());
        assert!(
            texels
                .flat_map(|f| &f.texels)
                .flatten()
                .all(|v| v.is_finite()),
            "every value finite"
        );
        let gpu = r.upload_environment(&lit);
        let view = View {
            occlusion: Occlusion::OFF,
            light: Light {
                environment: Some(&gpu),
                lights: &Lights::DEFAULT,
            },
            ..front_ortho(Shading::Lit(clay_material()))
        };
        snap(r, &mesh, &view, 128)
    };
    let darkest = [1, 1, 1, 1];
    let behind = |background| radiance_file(background, &[(0, 16, UNDER)]);
    for (what, bytes) in [
        ("over 1.0, behind", behind(ONE)),
        ("over RGBE's darkest, behind", behind(darkest)),
    ] {
        let shot = picture(&bytes);
        let black = (0..128u32)
            .flat_map(|y| (0..128u32).map(move |x| (x, y)))
            .filter(|&(x, y)| libm::hypotf(x as f32 - 63.5, y as f32 - 63.5) < 50.0)
            .filter(|&(x, y)| luma(shot.pixel(x, y)) == 0)
            .count();
        assert_eq!(black, 0, "{what}: black pixels on the sphere");
    }
    let (worst, _) = diff(
        &picture(&radiance_file(darkest, &[])),
        &picture(&radiance_file(ONE, &[])),
    );
    assert!(
        worst <= 1,
        "RGBE's darkest everywhere meters to what 1.0 everywhere does: {worst} levels apart"
    );
}

/// Review 0005, S1: the header's size is checked against the bytes left before memory is
/// reserved for it; a 16384 by 16384 claim in a few bytes once reserved 3 GiB.
#[test]
fn a_file_too_short_for_its_size_is_refused_before_its_pixels_are_reserved() {
    let mut bytes = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 16384 +X 16384\n".to_vec();
    bytes.extend([2, 2, 64, 0, 0x80 + 127, 0]);
    let refused = Equirect::read_hdr(&bytes)
        .expect_err("too short")
        .to_string();
    assert!(
        refused.contains("too short for an image of 16384 by 16384"),
        "{refused}"
    );
}
