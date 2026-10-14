//! The display step (research 11, section 6): view transforms on the CPU, and the GPU display
//! pass against them.

use crate::common::{BACKGROUND, front_ortho, lavapipe, snap, upload};
use crate::scene;
use gs_render::display::{decode, encode};
use gs_render::matcap::MatCapImage;
use gs_render::{Display, Shading, View, ViewTransform};

/// Hue angle around the white axis, in radians.
fn hue(c: [f32; 3]) -> f32 {
    libm::atan2f(3f32.sqrt() * (c[1] - c[2]), 2.0 * c[0] - c[1] - c[2])
}

/// PBR Neutral keeps every color's hue, in the linear part and the highlight shoulder alike.
#[test]
fn pbr_neutral_keeps_hue() {
    let levels: Vec<u8> = (0..=255).step_by(15).collect();
    let mut checked = 0;
    for &r in &levels {
        for &g in &levels {
            for &b in &levels {
                let light = decode([r, g, b]).map(|v| v * 3.0);
                let (hi, lo) = (light[0].max(light[1]).max(light[2]), light[0].min(light[1]));
                if hi - lo.min(light[2]) < 0.05 {
                    continue;
                }
                let shown = ViewTransform::PbrNeutral.apply(light);
                let turn = (hue(shown) - hue(light)).abs();
                let turn = turn.min(std::f32::consts::TAU - turn);
                assert!(turn < 2e-3, "{light:?} turns {turn} radians to {shown:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 3000, "most colors were checked ({checked})");
}

/// A display-referred (8-bit) MatCap shows exactly as its file at exposure 0, whatever the
/// view transform, and exposure scales its light: a camera-facing sphere shaded by a
/// one-color image shows that color at its center, and at -1.5 stops that color's light
/// times 2^-1.5, within one level. Dither moves covered pixels by at most one level and never
/// the background.
#[test]
fn an_8_bit_matcap_shows_as_its_file() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(4));
    for color in [
        [200u8, 60, 30],
        [255, 0, 0],
        [40, 120, 210],
        [250, 250, 245],
    ] {
        let level = color.map(|c| u16::from(c) * 257);
        let image = MatCapImage {
            size: 4,
            pixels: vec![[level[0], level[1], level[2], u16::MAX]; 16],
        };
        let look = r.upload_matcap(&image);
        let shown = |display| {
            let view = View {
                display,
                ..front_ortho(Shading::MatCap(&look))
            };
            snap(r, &mesh, &view, 32)
        };
        for transform in ViewTransform::ALL {
            for exposure in [0.0f32, -1.5] {
                let display = Display {
                    exposure,
                    transform,
                    dither: false,
                };
                let got = shown(display).pixel(16, 16);
                let want = encode(decode(color).map(|c| c * libm::exp2f(exposure)));
                let off = (0..3).map(|k| got[k].abs_diff(want[k])).max();
                assert!(
                    off <= Some(1),
                    "{color:?} {transform:?} at {exposure}: {got:?} against {want:?}"
                );
            }
        }
        let dithered = shown(Display::default());
        let plain = shown(Display {
            dither: false,
            ..Display::default()
        });
        for (p, q) in dithered.rgba.chunks(4).zip(plain.rgba.chunks(4)) {
            let off = (0..3).map(|k| p[k].abs_diff(q[k])).max();
            assert!(off <= Some(1), "dither moves {q:?} to {p:?}");
            if q == BACKGROUND {
                assert_eq!(p, BACKGROUND, "the background is never dithered");
            }
        }
    }
}
