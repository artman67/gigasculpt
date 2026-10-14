//! Catalog `brush-settings.curve`, its last clauses (decision 0059): Curve By Pen follows the
//! Zero Curve at no pressure and the Edit Curve (the tip's profile) at full pressure, and in
//! between blends them as the Pen Curve says, against the slow reference; Accurate Curve reads a
//! smooth curve exactly where its table cannot; the four profile presets follow their formulas;
//! the curves' values are checked by key, and a preset leaving the settings at their defaults
//! keeps its hash.

use gs_core::brush::Patch;
use gs_core::{BrushPreset, StrokeSample, Vec3};
use gs_protocol::PlainMesh;
use gs_testkit::reference::stroke::{Curve as Slow, Dab, dab};

use crate::brush_kit::{at, params, preset, run, shipped, store};
use crate::common::{ok, sphere, surface_point};

const HEAD: &str = "name = \"X\"\nid = \"x\"\nfamily = \"Build\"\nstrength = 0.6\n[kernel]\n\
                    type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.5 }]\n";

/// The Edit Curve and the Zero Curve blended: a high shoulder and a narrow peak, straight
/// between their points.
const EDIT: [[f64; 2]; 3] = [[0.0, 1.0], [0.6, 0.8], [1.0, 0.0]];
const ZERO: [[f64; 2]; 3] = [[0.0, 1.0], [0.3, 0.1], [1.0, 0.0]];

/// Full strength at every pressure, so a light press still sculpts and only the shape changes.
const FLAT: &str = "[pressure]\nstrength = [[0, 1], [1, 1]]\n";

/// `[tip]` lines: a custom profile through `points`, then `more`.
fn tip(points: &[[f64; 2]], more: &str) -> String {
    format!("[tip]\nprofile = \"custom\"\npoints = {points:?}\n{more}\n")
}

/// The shipped preset `stem` with `patch`.
fn patched(stem: &str, patch: &str) -> BrushPreset {
    let patch = ok(toml::from_str::<Patch>(patch));
    ok(BrushPreset::resolve(&preset(stem).text, &patch))
}

/// The sphere the strokes land on, and the point they press.
fn ball() -> (PlainMesh, Vec3) {
    let mesh = sphere(10, 0.5, 91, false);
    let g = surface_point(&store(&mesh), Vec3::new(0.4, 0.5, 0.8));
    (mesh, g)
}

/// The positions after a two-sample stroke of `p` at `pressure`: dabs, or a drag update.
fn stroke_at(p: &BrushPreset, pressure: f32) -> Vec<[f32; 3]> {
    let (mesh, g) = ball();
    let sample = |pos| StrokeSample {
        pressure,
        ..at(pos, 0.35)
    };
    let mut m = store(&mesh);
    run(
        &mut m,
        p,
        params(None),
        &[sample(g), sample(g + Vec3::new(-0.03, 0.04, 0.01))],
    );
    m.to_plain().positions
}

#[test]
fn curve_by_pen_follows_the_zero_curve_at_no_pressure_and_the_edit_curve_at_full() {
    for stem in ["draw", "move"] {
        let by_pen = format!("curve_by_pen = true\nzero_curve = {ZERO:?}");
        let on = patched(stem, &format!("{}{FLAT}", tip(&EDIT, &by_pen)));
        let edit = patched(stem, &format!("{}{FLAT}", tip(&EDIT, "")));
        let zero = patched(stem, &format!("{}{FLAT}", tip(&ZERO, "")));
        let light = stroke_at(&on, 0.0);
        assert_eq!(
            light,
            stroke_at(&zero, 0.0),
            "{stem}: no pressure is the Zero Curve"
        );
        assert_ne!(
            light,
            stroke_at(&edit, 0.0),
            "{stem}: the two curves differ"
        );
        assert_eq!(
            stroke_at(&on, 1.0),
            stroke_at(&edit, 1.0),
            "{stem}: full pressure is the Edit Curve"
        );
    }
}

#[test]
fn between_them_the_pen_curve_blends_the_two_as_the_slow_reference_does() {
    // At pressure 0.5 the Pen Curve gives 0.2: a fifth of the way from the Zero Curve to the
    // Edit Curve.
    let (pressure, share) = (0.5, 0.2);
    let by_pen = format!(
        "curve_by_pen = true\nzero_curve = {ZERO:?}\npen_curve = [[0, 0], [0.5, {share}], [1, 1]]"
    );
    let text = format!("{HEAD}{}{FLAT}", tip(&EDIT, &by_pen));
    // Both curves are straight between their points, so the blend is straight between the
    // points of both: the reference reads it as a plain custom profile.
    let sharp = |c: &[[f64; 2]]| Slow::new(c.to_vec(), vec![false; c.len()]);
    let (e, z) = (sharp(&EDIT), sharp(&ZERO));
    let blend: Vec<[f64; 2]> = [0.0, 0.3, 0.6, 1.0]
        .iter()
        .map(|&x| [x, z.at(x) + share * (e.at(x) - z.at(x))])
        .collect();
    let slow = format!("{HEAD}{}{FLAT}", tip(&blend, ""));

    let (mesh, g) = ball();
    let mut m = store(&mesh);
    let before = m.to_plain();
    let view = (Vec3::ZERO - g).normalized();
    let mut prm = params(None);
    prm.view.dir = view;
    let sample = StrokeSample {
        pressure,
        ..at(g, 0.35)
    };
    run(&mut m, &ok(BrushPreset::from_toml(&text)), prm, &[sample]);
    let d = |v: Vec3| v.to_array().map(f64::from);
    let want = ok(dab(
        &slow,
        &before,
        &Dab {
            alpha: None,
            at: d(g),
            to: None,
            radius: 0.35,
            pressure: f64::from(pressure),
            view: d(view),
            strength: None,
            invert: false,
        },
    ));
    let (mut worst, mut change) = (0.0f64, 0.0f64);
    let got = m.to_plain().positions;
    for ((g, w), b) in got.iter().zip(&want.positions).zip(&before.positions) {
        for k in 0..3 {
            worst = worst.max((f64::from(g[k]) - w[k]).abs());
            change = change.max((w[k] - f64::from(b[k])).abs());
        }
    }
    assert!(worst <= 3e-6, "{worst} from the reference");
    assert!(change > 1e-3, "the dab did something: {change}");
}

#[test]
fn accurate_curve_reads_a_smooth_curve_exactly_where_its_table_cannot() {
    // A smooth ring half a table step wide (1/512 of the radius) just past half the radius:
    // the table's entries at 128/256 and 129/256 both read 0 there.
    let ring = [
        [0.0, 0.0],
        [0.5, 0.0],
        [0.500_976_562_5, 1.0],
        [0.501_953_125, 0.0],
        [1.0, 0.0],
    ];
    let points = format!("{{ points = {ring:?}, smooth = true }}");
    let tip = |more: &str| {
        let text = format!("{HEAD}[tip]\nprofile = \"custom\"\npoints = {points}\n{more}\n");
        ok(BrushPreset::from_toml(&text)).tip.tip()
    };
    let (off, on) = (tip(""), tip("accurate_curve = true"));
    let exact = Slow::new(ring.to_vec(), vec![true; ring.len()]);
    for i in 0..4096 {
        let d = i as f32 / 4096.0;
        let want = exact.cubic(f64::from(d));
        let got = f64::from(on.falloff(d));
        assert!((got - want).abs() < 1e-6, "on, at {d}: {got} not {want}");
    }
    let peak = 0.500_976_6;
    assert!(on.falloff(peak) > 0.999, "on reads the ring's peak");
    assert!(off.falloff(peak) < 1e-6, "off, the table steps over it");
}

#[test]
fn the_four_profile_presets_follow_their_formulas() {
    type Case<'a> = (&'a str, &'a dyn Fn(f64) -> f64);
    let sq = |x: f64| x * x;
    let cases: [Case; 4] = [
        ("soft", &|d| 1.0 - 3.0 * d * d + 2.0 * d * d * d),
        ("hard", &|d| 1.0 - sq(sq(sq(d)))),
        ("flat_top", &|d| sq(1.0 - sq(sq(d)))),
        ("sharp_center", &|d| sq(sq(1.0 - d))),
    ];
    for (name, want) in cases {
        let text = format!("{HEAD}[tip]\nprofile = \"{name}\"\n");
        let tip = ok(BrushPreset::from_toml(&text)).tip.tip();
        for i in 0..=40 {
            let d = f64::from(i) / 32.0;
            let w = if d >= 1.0 { 0.0 } else { want(d) };
            let got = f64::from(tip.falloff(d as f32));
            assert!((got - w).abs() < 2e-6, "{name} at {d}: {got} not {w}");
        }
    }
}

#[test]
fn the_curves_are_checked_by_key_and_the_defaults_keep_every_hash() {
    for (patch, key) in [
        (
            "[tip]\nzero_curve = [[0, 1.5], [1, 0]]",
            "tip.zero_curve values must be from 0 to 1",
        ),
        (
            "[tip]\npen_curve = [[0, -0.2], [1, 1]]",
            "tip.pen_curve values must be from 0 to 1",
        ),
    ] {
        let bad = ok(toml::from_str::<Patch>(patch));
        let why = BrushPreset::resolve(&preset("draw").text, &bad).map_err(|e| e.message);
        assert!(
            why.as_ref().is_err_and(|m| m.contains(key)),
            "{patch}: {:?}",
            why.err()
        );
    }
    // Every shipped preset leaves them out, and writing them at their defaults changes no hash.
    let new = ["accurate_curve", "curve_by_pen", "zero_curve", "pen_curve"];
    let defaults = "[tip]\naccurate_curve = false\ncurve_by_pen = false\n\
                    zero_curve = [[0, 1], [1, 0]]\npen_curve = [[0, 0], [1, 1]]\n";
    let defaults = ok(toml::from_str::<Patch>(defaults));
    for s in shipped() {
        let text = s.preset.canonical();
        assert!(new.iter().all(|k| !text.contains(k)), "{}: {text}", s.stem);
        let same = ok(BrushPreset::resolve(&s.text, &defaults));
        assert_eq!(same.hash(), s.preset.hash(), "{}", s.stem);
    }
    // Set, each is written, and the text reads back as the same brush.
    let set = patched(
        "draw",
        "[tip]\nprofile = \"sharp_center\"\naccurate_curve = true\ncurve_by_pen = true\n\
         zero_curve = [[0, 1], [0.25, 0.5], [1, 0]]\n\
         pen_curve = { points = [[0, 0], [0.5, 0.75], [1, 1]], smooth = true }",
    );
    let text = set.canonical();
    for line in [
        "profile = \"sharp_center\"",
        "accurate_curve = true",
        "curve_by_pen = true",
        "zero_curve = ",
        "pen_curve",
    ] {
        assert!(text.contains(line), "{line}: {text}");
    }
    assert_eq!(ok(BrushPreset::from_toml(&text)), set, "reads back");
}
