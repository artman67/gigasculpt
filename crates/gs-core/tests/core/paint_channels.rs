//! Paint channels in presets (decision 0039, section 8): a `[paint]` table lets any brush paint
//! color, alone (mode off) or while it sculpts, after its geometry kernel and auto-smooth in each
//! dab; Color Strength caps what a stroke paints; Smooth with `[paint]` averages color in linear
//! light.

use gs_core::{BrushPreset, Vec3};
use gs_protocol::{AttrValues, PlainMesh};

use crate::brush_kit::{
    RED, after, at, colors, ico, mirror_x, params, polyline, preset, samples, store,
};
use crate::common::ok;
use crate::paint_stroke::{decode, encode, plane};

const BLACK: u32 = 0xff00_0000;
const WHITE: u32 = u32::MAX;

/// Channel `k` of color `c`, in levels.
fn ch(c: u32, k: u32) -> f64 {
    f64::from(c >> (8 * k) & 255)
}

/// The shipped brush `stem` with `extra` added to its `[paint]` table, or with a `[paint]` table
/// painting color and holding `extra` if it has none.
fn with_paint(stem: &str, extra: &str) -> BrushPreset {
    let text = preset(stem).text;
    let text = if text.contains("[paint]\n") {
        text.replace("[paint]\n", &format!("[paint]\n{extra}\n"))
    } else {
        format!("{text}\n[paint]\nchannel = \"color\"\n{extra}\n")
    };
    ok(BrushPreset::from_toml(&text))
}

#[test]
fn color_strength_caps_a_stroke_however_many_dabs_overlap() {
    let mesh = plane(BLACK);
    let mut prm = params(None);
    prm.colors.main = WHITE;
    // Back and forth four times along the plane, each pass covering a point many times over.
    let (a, b) = (Vec3::new(-0.6, 0.05, 0.0), Vec3::new(0.6, 0.05, 0.0));
    let path = polyline(
        &[
            (a, 1.0, 0.3),
            (b, 1.0, 0.3),
            (a, 1.0, 0.3),
            (b, 1.0, 0.3),
            (a, 1.0, 0.3),
        ],
        32,
    );
    let out = after(
        &mesh,
        &with_paint("paint-draw", "color_strength = 0.5"),
        prm,
        &path,
    );
    // Half way from black to white in linear light, 187.5 levels in sRGB (at 50% in sRGB it
    // would be 127.5): the stroke's end rounds it with a dither to 187 or 188.
    let half = encode(0.5);
    assert!((half - 187.5).abs() < 0.1, "{half}");
    let mut covered = 0;
    for (p, &c) in out.positions.iter().zip(&colors(&out)) {
        let under = (p[1] - 0.05).abs() < 0.1 && p[0].abs() < 0.5;
        for k in 0..3 {
            let level = ch(c, k);
            assert!(level <= 188.0, "{p:?}: {c:08x} went past half");
            if under {
                assert!((187.0..=188.0).contains(&level), "{p:?}: {c:08x}");
            }
        }
        covered += usize::from(under);
    }
    assert!(covered > 100, "{covered} points under the line");
}

#[test]
fn a_brush_paints_alone_or_while_it_sculpts_and_painting_never_moves_a_point() {
    let mesh = store(&ico(4)).to_plain();
    let prm = params(Some(mirror_x()));
    let draw = preset("draw").preset;
    let smp = samples(&draw, 0.3);
    let drawn = after(&mesh, &draw, prm.clone(), &smp);
    let both = after(&mesh, &with_paint("draw", ""), prm.clone(), &smp);
    let alone = after(&mesh, &preset("paint-draw").preset, prm, &smp);
    let bits = |m: &PlainMesh| -> Vec<[u32; 3]> {
        m.positions.iter().map(|p| p.map(f32::to_bits)).collect()
    };
    assert_eq!(bits(&both), bits(&drawn), "Draw painting draws as Draw");
    assert_eq!(bits(&alone), bits(&mesh), "Paint moves no point");
    assert!(
        colors(&drawn).iter().all(|&c| c == WHITE),
        "Draw paints nothing"
    );
    // From white toward red: red stays 255, green and blue fall alike.
    for (name, m) in [("Draw painting", &both), ("Paint", &alone)] {
        let c = colors(m);
        let reddened = (c.iter().filter(|&&c| c != WHITE))
            .inspect(|&&c| assert_eq!(c & 0xff00_00ff, RED, "{name}: {c:08x}"))
            .filter(|&&c| ch(c, 1) < 200.0)
            .count();
        assert!(reddened > 10, "{name} painted {reddened} points toward red");
    }
}

#[test]
fn smooth_with_paint_averages_color_in_linear_light() {
    // A white plane with one red point in its middle, under one dab of radius 0.3.
    let mut mesh = plane(WHITE);
    let dot = (mesh.positions.iter())
        .position(|p| p.map(f32::to_bits) == [0; 3])
        .unwrap_or_default();
    if let Some(AttrValues::U32(c)) = mesh.attributes.get_mut("color").map(|a| &mut a.values) {
        c[dot] = RED;
    }
    let bell = |d: f64| 3.0 * d * d * d * d - 4.0 * d * d * d + 1.0;
    let h = 2.0 / 64.0;
    for (name, p) in [
        ("Smooth", with_paint("smooth", "")),
        ("Paint Blend", preset("paint-blend").preset),
    ] {
        let out = after(&mesh, &p, params(None), &[at(Vec3::ZERO, 0.3)]);
        let got = colors(&out);
        if name == "Paint Blend" {
            let bits = |m: &PlainMesh| -> Vec<[u32; 3]> {
                m.positions.iter().map(|p| p.map(f32::to_bits)).collect()
            };
            assert_eq!(bits(&out), bits(&mesh), "Paint Blend moves no point");
        }
        // Smooth's step at gain 2 for one dab of amount 0.5 (tap 0.5, strength 1).
        let frac = |d: f64| 1.0 - libm::exp(-bell(d / 0.3));
        // The red point goes toward its white neighbours' mean.
        let want = encode(frac(0.0));
        let c = got[dot];
        assert_eq!(c & 0xff, 255, "{name}: red stays");
        for k in 1..3 {
            assert!(
                (ch(c, k) - want).abs() < 1.0,
                "{name}: the dot {c:08x}, {want}"
            );
        }
        // Each of its four neighbours goes toward a mean of one red and three white: 0.75 in
        // linear light for green and blue, where an average of the bytes would give 0.75 of 255.
        let mean = (decode(0.0) + 3.0 * decode(255.0)) / 4.0;
        let want = encode(1.0 + (mean - 1.0) * frac(h));
        assert!(want > 236.0, "linear light, not bytes: {want}");
        let mut pink = 0;
        for (p, &c) in out.positions.iter().zip(&got) {
            let d = f64::from(p[0] * p[0] + p[1] * p[1]).sqrt();
            if (d - h).abs() < 1e-4 {
                assert_eq!(c & 0xff, 255, "{name}: red stays");
                for k in 1..3 {
                    assert!(
                        (ch(c, k) - want).abs() < 1.0,
                        "{name} {p:?}: {c:08x}, {want}"
                    );
                }
                pink += 1;
            }
        }
        assert_eq!(pink, 4, "{name}: the dot's four neighbours turn pink");
    }
}
