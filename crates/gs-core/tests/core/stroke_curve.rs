//! The curve line's points (decision 0055, `stroke.curve-line`): each Alt tap adds a soft point
//! the path bends through and a double tap makes it a sharp corner, Space held moves the whole
//! path, Alt at the last sample still flips Clip Curve's shadow side, and Clay Line lays its
//! dabs along the same path.

use gs_core::brush::{BrushPreset, Smoothing, StrokeSpec};
use gs_core::stroke::curve_path;
use gs_core::{MeshStore, Stroke, StrokeSample, Vec3};
use gs_protocol::PlainMesh;
use gs_testkit::compare::canonical;

use crate::brush_kit::{at, ico, params, preset, shipped_alpha, store, to_segment};
use crate::common::ok;

/// What the sculptor did after a sample: nothing, one Alt tap, or a double tap.
#[derive(Clone, Copy, PartialEq)]
enum Tap {
    No,
    Soft,
    Sharp,
}

/// A sample on the plane z = 2, in front of [`mesh`].
fn p(x: f32, y: f32) -> StrokeSample {
    at(Vec3::new(x, y, 2.0), 0.1)
}

/// A sphere of radius 1 moved back from the plane the curves are drawn on.
fn mesh() -> PlainMesh {
    let mut m = ico(4);
    for q in &mut m.positions {
        q[2] -= 4.0;
    }
    m
}

/// A stroke of `brush` along `samples` on `m`: `taps[i]` after sample `i`, the samples in
/// `moving` fed with Space held, and Alt held at the last sample when `alt`. Returns the
/// stroke's points as it ended.
fn draw(
    m: &mut MeshStore,
    brush: &BrushPreset,
    samples: &[StrokeSample],
    taps: &[Tap],
    moving: &[usize],
    alt: bool,
) -> Vec<(StrokeSample, bool)> {
    let mut prm = params(None);
    prm.alpha = shipped_alpha(brush);
    let mut k = ok(Stroke::begin(m, brush, prm));
    for (i, s) in samples.iter().enumerate() {
        k.set_invert(alt && i + 1 == samples.len());
        k.set_moving(moving.contains(&i));
        ok(k.add(m, &[*s]));
        match taps.get(i) {
            Some(Tap::Soft) => k.point(),
            Some(Tap::Sharp) => {
                k.point();
                k.corner();
            }
            _ => {}
        }
    }
    let points = k.path();
    k.finish(m);
    points
}

/// Checks that every vertex of `before` on the shadow side of its nearest segment of `line`
/// (as the front camera sees it; the other side with `alt`) landed on the plane that segment
/// sweeps along the view in `after`, and every other stayed. Returns how many moved.
fn cut_along(before: &PlainMesh, after: &PlainMesh, line: &[StrokeSample], alt: bool) -> usize {
    let flat = |v: Vec3| Vec3::new(v.x, v.y, 0.0);
    let cuts: Vec<(Vec3, Vec3, Vec3)> = (line.windows(2))
        .map(|s| {
            let n = (s[1].pos - s[0].pos)
                .cross(Vec3::new(0.0, 0.0, -1.0))
                .normalized();
            (s[0].pos, s[1].pos, if alt { -n } else { n })
        })
        .collect();
    let mut moved = 0;
    for (b, a) in before.positions.iter().zip(&after.positions) {
        let q = Vec3::from_array(*b);
        let mut near: Vec<(f32, usize)> = (cuts.iter().enumerate())
            .map(|(i, c)| (to_segment(flat(q), flat(c.0), flat(c.1)), i))
            .collect();
        near.sort_by(|x, y| x.0.total_cmp(&y.0));
        if near.len() > 1 && near[1].0 - near[0].0 < 1e-4 {
            // Equally near two segments: rounding picks one, so skip it.
            continue;
        }
        let (o, _, n) = cuts[near[0].1];
        let h = (q - o).dot(n);
        let want = if h > 0.0 { q - n * h } else { q };
        assert!(
            (Vec3::from_array(*a) - want).len2().sqrt() < 1e-5,
            "{q:?} went to {a:?}, not {want:?}"
        );
        moved += usize::from(h > 0.0);
    }
    moved
}

#[test]
fn tapping_alt_n_times_inserts_n_points_and_the_shadow_side_flips_with_alt() {
    let clip = preset("clip-curve").preset;
    let pen = [
        p(-2.0, 0.5),
        p(-1.0, -0.25),
        p(0.0, 0.5),
        p(1.0, -0.25),
        p(2.0, 0.25),
    ];
    for n in 0..=3 {
        let taps: Vec<Tap> = (0..pen.len())
            .map(|i| {
                if (1..=n).contains(&i) {
                    Tap::Soft
                } else {
                    Tap::No
                }
            })
            .collect();
        let mut shadow = Vec::new();
        for alt in [false, true] {
            let mut m = store(&mesh());
            let before = m.to_plain();
            let points = draw(&mut m, &clip, &pen, &taps, &[], alt);
            let soft: Vec<Vec3> = (points.iter()).filter(|q| !q.1).map(|q| q.0.pos).collect();
            let tapped: Vec<Vec3> = pen[1..=n].iter().map(|s| s.pos).collect();
            assert_eq!(soft, tapped, "{n} taps: {n} soft points, where the pen was");
            assert_eq!(points.len(), n + 2, "and the press and the pen");
            let after = m.to_plain();
            let moved = cut_along(&before, &after, &curve_path(&points), alt);
            assert!(moved > 100, "{n} taps, Alt {alt}: the cut moved {moved}");
            shadow.push(canonical(&after));
        }
        assert!(shadow[0] != shadow[1], "{n} taps: Alt cuts the other side");
    }
}

#[test]
fn a_double_tap_makes_a_sharp_corner_where_one_tap_bends_smoothly() {
    let (a, b, c) = (p(-2.0, 0.5), p(0.0, -0.5), p(2.0, 0.5));
    let sharp = curve_path(&[(a, true), (b, true), (c, true)]);
    let pos: Vec<Vec3> = sharp.iter().map(|s| s.pos).collect();
    assert_eq!(
        pos,
        [a.pos, b.pos, c.pos],
        "corners only: straight segments"
    );
    let soft = curve_path(&[(a, true), (b, false), (c, true)]);
    let i = soft.iter().position(|s| s.pos == b.pos);
    let i = i.unwrap_or_else(|| panic!("the curve passes through its point"));
    let turn = |path: &[StrokeSample], i: usize| {
        let u = (path[i].pos - path[i - 1].pos).normalized();
        u.dot((path[i + 1].pos - path[i].pos).normalized())
    };
    assert!(turn(&soft, i) > 0.98, "a soft point: no corner");
    assert!(turn(&sharp, 1) < 0.7, "a sharp point corners");
    let clip = preset("clip-curve").preset;
    let mut m = store(&mesh());
    let before = m.to_plain();
    let taps = [Tap::No, Tap::Sharp, Tap::No];
    let points = draw(&mut m, &clip, &[a, b, c], &taps, &[], false);
    let kinds: Vec<bool> = points.iter().map(|q| q.1).collect();
    assert_eq!(kinds, [true; 3], "the double tap's point is sharp");
    let moved = cut_along(&before, &m.to_plain(), &[a, b, c], false);
    assert!(moved > 100, "the cut follows the corner ({moved})");
}

#[test]
fn space_held_moves_the_whole_path_with_the_pen() {
    let clip = preset("clip-curve").preset;
    // The press, a soft point, on; Space held while the pen moves half right and a quarter up;
    // then on to the end.
    let drawn = [
        p(-2.0, 0.5),
        p(-0.5, -0.5),
        p(0.5, 0.25),
        p(1.0, 0.25),
        p(1.0, 0.5),
        p(2.0, 0.0),
    ];
    let taps = [Tap::No, Tap::Soft];
    let mut m = store(&mesh());
    let points = draw(&mut m, &clip, &drawn, &taps, &[3, 4], false);
    let got: Vec<([f32; 3], bool)> = (points.iter()).map(|q| (q.0.pos.to_array(), q.1)).collect();
    let want = [
        ([-1.5, 0.75, 2.0], true),
        ([0.0, -0.25, 2.0], false),
        ([2.0, 0.0, 2.0], true),
    ];
    assert_eq!(got, want, "the press and the point moved with the pen");
    let moved = [p(-1.5, 0.75), p(0.0, -0.25), p(1.0, 0.5), p(2.0, 0.0)];
    let mut direct = store(&mesh());
    draw(&mut direct, &clip, &moved, &taps, &[], false);
    assert!(
        canonical(&m.to_plain()) == canonical(&direct.to_plain()),
        "the same cut as the path drawn where it was moved to"
    );
}

#[test]
fn clay_line_lays_its_dabs_along_the_bent_path() {
    let line = preset("clay-line").preset;
    let StrokeSpec::Curve { spacing, .. } = line.stroke else {
        panic!("Clay Line is a curve stroke");
    };
    let dabs = BrushPreset {
        stroke: StrokeSpec::Dab {
            spacing,
            tap: 0.0,
            smoothing: Smoothing::default(),
            directional: false,
        },
        ..line.clone()
    };
    let top = |x: f32, y: f32| at(Vec3::new(x, y, 1.0).normalized(), 0.2);
    let pen = [top(0.3, 0.0), top(0.1, 0.3), top(-0.2, 0.0)];
    let mut bent = store(&ico(4));
    let points = draw(&mut bent, &line, &pen, &[Tap::No, Tap::Soft], &[], false);
    let mut along = store(&ico(4));
    draw(&mut along, &dabs, &curve_path(&points), &[], &[], false);
    let bent = canonical(&bent.to_plain());
    assert!(bent == canonical(&along.to_plain()), "dabs along the curve");
    let mut straight = store(&ico(4));
    draw(&mut straight, &line, &pen, &[], &[], false);
    assert!(
        bent != canonical(&straight.to_plain()),
        "not the straight line"
    );
}
