//! The paint stroke (decision 0039, section 6): color mixes in linear light in a float buffer
//! held for the stroke and rounds to 8 bits once, at the stroke's end, with a dither from the
//! stroke's own seed. So soft strokes do not stall (research 24, wrong result 2), and the
//! result is the same on any thread count and replays exactly.

use gs_core::rng::CommandRng;
use gs_core::{BrushPreset, MeshStore, StrokeParams, Vec3};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::brush_kit::{colors, floats, grid, params, polyline, preset, run, store};
use crate::common::ok;

const GREY: u32 = 0xff64_6464;
const TINT: u32 = 0xff8c_8c8c;

/// The shipped brush `stem` at strength 0.01: the Paint brush, or the Mask Pen, whose mask
/// mixes the same way in floats (both gain 8 on the smooth tip, spacing 0.1, tap 0.5).
fn soft(stem: &str) -> BrushPreset {
    let text = preset(stem)
        .text
        .replace("strength = 1.0", "strength = 0.01");
    ok(BrushPreset::from_toml(&text))
}

/// A plane of 65 by 65 points, every point colored `color`.
pub fn plane(color: u32) -> PlainMesh {
    let mut mesh = store(&grid(64)).to_plain();
    let n = mesh.positions.len();
    mesh.attributes.insert(
        "color".into(),
        Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::U32(vec![color; n]),
        },
    );
    mesh
}

/// A straight stroke across the plane at full pressure.
fn line() -> Vec<gs_core::StrokeSample> {
    let (a, b) = (Vec3::new(-0.8, 0.05, 0.0), Vec3::new(0.8, 0.05, 0.0));
    polyline(&[(a, 1.0, 0.3), (b, 1.0, 0.3)], 32)
}

/// `count` strokes of `p` along the line on `s`, painting `TINT`, each with its own seed, as
/// the engine gives each stroke command its own stream.
fn strokes(s: &mut MeshStore, p: &BrushPreset, count: u64) {
    for i in 0..count {
        let mut prm = StrokeParams {
            seed: CommandRng::for_command(1, i).seed(),
            ..params(None)
        };
        prm.colors.main = TINT;
        run(s, p, prm, &line());
    }
}

/// An 8-bit sRGB level (0 to 255, fractional allowed) in linear light, and back.
pub fn decode(level: f64) -> f64 {
    let c = level / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        libm::pow((c + 0.055) / 1.055, 2.4)
    }
}
pub fn encode(l: f64) -> f64 {
    255.0
        * if l <= 0.003_130_8 {
            l * 12.92
        } else {
            1.055 * libm::pow(l, 1.0 / 2.4) - 0.055
        }
}

/// Mean, 99th percentile and share above 3 levels of `errors`.
fn stats(mut errors: Vec<f64>) -> (f64, f64, f64) {
    errors.sort_by(f64::total_cmp);
    let n = errors.len().max(1) as f64;
    let mean = errors.iter().sum::<f64>() / n;
    let p99 = errors[((n * 0.99) as usize).min(errors.len() - 1)];
    let over = errors.iter().filter(|&&e| e > 3.0).count() as f64 / n;
    (mean, p99, over)
}

#[test]
fn twenty_soft_strokes_land_within_a_level_of_a_float_run() {
    let mesh = plane(GREY);
    let mut painted = store(&mesh);
    strokes(&mut painted, &soft("paint-draw"), 20);
    let got = colors(&painted.to_plain());
    // The same dabs on a mask column, which mixes in floats: after 20 strokes each point's
    // mask is the share of the way to the target a float run of the same mixing goes.
    let mut float = store(&mesh);
    strokes(&mut float, &soft("mask-pen"), 20);
    let m = floats(&float.to_plain(), "mask");
    let (lo, hi) = (decode(100.0), decode(140.0));
    let (mut linear, mut srgb) = (Vec::new(), Vec::new());
    for (i, &c) in got.iter().enumerate() {
        let m = f64::from(m[i]);
        let want = encode(lo + (hi - lo) * m);
        // Only points the float run moved by half a level or more.
        if want - 100.0 >= 0.5 {
            let byte = f64::from(c & 255);
            linear.push((byte - want).abs());
            srgb.push((byte - (100.0 + 40.0 * m)).abs());
        }
    }
    let moved = linear.len();
    let (mean, p99, over) = stats(linear);
    let (s_mean, s_p99, s_over) = stats(srgb);
    eprintln!(
        "{moved} points moved; against a linear float run: mean {mean:.2}, p99 {p99:.2}, \
         over 3 {:.1}%; against an sRGB float run: mean {s_mean:.2}, p99 {s_p99:.2}, over 3 \
         {:.1}%",
        over * 100.0,
        s_over * 100.0
    );
    assert!(moved > 500, "the strokes moved {moved} points");
    assert!(
        mean <= 1.5 && over <= 0.05,
        "mean {mean:.2} levels, {:.1}% over 3 levels from a float run in linear light",
        over * 100.0
    );
}

#[test]
fn a_paint_stroke_is_the_same_on_1_and_8_threads_and_replays_exactly() {
    let mesh = plane(GREY);
    let painter = soft("paint-draw");
    let paint = || {
        let mut s = store(&mesh);
        strokes(&mut s, &painter, 3);
        colors(&s.to_plain())
    };
    let one = gs_par::with_threads(1, paint);
    let eight = gs_par::with_threads(8, paint);
    assert!(one.iter().any(|&c| c != GREY), "the strokes painted");
    assert_eq!(one, eight, "1 and 8 threads");
    assert_eq!(gs_par::with_threads(8, paint), eight, "a replay");
}
