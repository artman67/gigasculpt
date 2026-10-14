//! Pinch's and Pull Out's Checks the claim harness does not cover (catalog `brush.pinch` and
//! `brush.pull-out`, decision 0049, section 8), each a named test: Pinch brings two grooves
//! together without changing the volume and adds points only inside its footprint, and Pull
//! Out's refined extrusion keeps its edges within twice the target.

use std::collections::BTreeSet;

use gs_core::live::{LiveDetail, Target};
use gs_core::{NO, StrokeSample, Vec3};
use gs_protocol::PlainMesh;

use crate::brush_checks::relief;
use crate::brush_kit::{at, grid, params, preset, run, store, to_segment};
use crate::common::some;

/// Two grooves along y, 0.05 deep and about two edges of the 96 grid wide, at x = -0.15 and
/// 0.15.
fn grooves(x: f32, _: f32) -> f32 {
    let g = |c: f32| {
        let d = (x - c) / 0.04;
        -0.05 * libm::expf(-d * d)
    };
    g(-0.15) + g(0.15)
}

/// One pass between the grooves: 25 full-pressure samples of radius 0.3 along x = 0 from
/// y = -0.6 to 0.6.
fn between() -> Vec<StrokeSample> {
    (0..=24)
        .map(|i| {
            let y = -0.6 + 1.2 * i as f32 / 24.0;
            at(Vec3::new(0.0, y, grooves(0.0, y)), 0.3)
        })
        .collect()
}

/// Every face's corners in order, without the unused fourth of a triangle.
fn corners(m: &PlainMesh) -> impl Iterator<Item = Vec<usize>> + '_ {
    (m.faces.iter()).map(|f| {
        (f.iter().filter(|&&v| v != NO))
            .map(|&v| v as usize)
            .collect()
    })
}

/// The volume between the surface and z = 0: each face, cut into triangles, by its area seen
/// from above times its mean height.
fn volume(m: &PlainMesh) -> f64 {
    let p = |i: usize| m.positions[i].map(f64::from);
    let mut v = 0.0;
    for c in corners(m) {
        for k in 1..c.len() - 1 {
            let (a, b, d) = (p(c[0]), p(c[k]), p(c[k + 1]));
            let area = 0.5 * ((b[0] - a[0]) * (d[1] - a[1]) - (b[1] - a[1]) * (d[0] - a[0]));
            v += area * (a[2] + b[2] + d[2]) / 3.0;
        }
    }
    v
}

/// Where along x the groove right of the stroke is deepest, read on the points that started on
/// the row y = 0.
fn groove(before: &PlainMesh, after: &PlainMesh) -> f32 {
    let row = (0..before.positions.len())
        .filter(|&i| before.positions[i][1] == 0.0 && before.positions[i][0] > 0.0);
    let deepest = row.min_by(|&i, &j| after.positions[i][2].total_cmp(&after.positions[j][2]));
    after.positions[some(deepest)][0]
}

/// The Check: two parallel grooves on a plane move closer together after one Pinch pass
/// between them, with no net volume change at Brush Modifier 0. Pinch moves points within the
/// surface, so the volume under it changes by far less than the volume its points swept, while
/// a Brush Modifier of -20 (an inflate term) on the same stroke adds volume.
#[test]
fn pinch_brings_two_grooves_together_and_keeps_the_volume() {
    let before = relief(96, &grooves);
    let pinch = preset("pinch").preset;
    let pinched = |modifier: f32| {
        let mut p = pinch.clone();
        p.modifier = modifier;
        let mut s = store(&before);
        run(&mut s, &p, params(None), &between());
        s.to_plain()
    };
    let (after, inflated) = (pinched(0.0), pinched(-20.0));
    let (from, to) = (groove(&before, &before), groove(&before, &after));
    let side = f64::from(2.0f32 / 96.0);
    let cell = side * side;
    let swept: f64 = (before.positions.iter().zip(&after.positions))
        .map(|(a, b)| f64::from((Vec3::from_array(*a) - Vec3::from_array(*b)).len2().sqrt()))
        .sum::<f64>()
        * cell;
    let v0 = volume(&before);
    let (net, more) = (volume(&after) - v0, volume(&inflated) - v0);
    eprintln!(
        "groove from x = {from} to {to}; volume {v0}, changed {net} by Pinch, {more} at \
         modifier -20; the moved points swept {swept}"
    );
    assert!(to < from - 0.01, "the grooves moved closer: {from} to {to}");
    assert!(
        net.abs() < 0.01 * swept,
        "the volume changed by {net} of {swept} swept"
    );
    assert!(more > 10.0 * net.abs(), "the inflate term adds {more}");
}

/// The Check: with Live Detail off Pinch keeps the vertex count, with it on the count rises
/// inside the footprint. The points within the brush radius of the stroke grow in number, and
/// every point the refined stroke made or moved lies within that radius plus two edges: a split
/// lands on a face the dab touches, which may reach an edge out, and longest-edge bisection may
/// first split the longer side of the face beyond it (decision 0021).
#[test]
fn pinch_adds_points_only_with_live_detail_and_only_inside_its_footprint() {
    let before = relief(48, &grooves);
    let pinch = preset("pinch").preset;
    let path = between();
    let edge = 2.0 / 48.0;
    let mut off = store(&before);
    run(&mut off, &pinch, params(None), &path);
    let mut on = store(&before);
    let mut prm = params(None);
    prm.live_detail = Some(LiveDetail {
        target: Target::Constant(edge / 2.0),
        ..LiveDetail::default()
    });
    run(&mut on, &pinch, prm, &path);
    let (off, on) = (off.to_plain(), on.to_plain());
    let old: BTreeSet<[u32; 3]> = (before.positions.iter())
        .map(|p| p.map(f32::to_bits))
        .collect();
    let (a, b) = (path[0].pos, path[path.len() - 1].pos);
    let from = |p: &[f32; 3]| to_segment(Vec3::from_array(*p), a, b);
    let inside = |m: &PlainMesh| m.positions.iter().filter(|p| from(p) <= 0.3).count();
    let far = (on.positions.iter())
        .filter(|p| !old.contains(&p.map(f32::to_bits)))
        .map(from)
        .fold(0.0, f32::max);
    let n = before.positions.len();
    eprintln!(
        "{n} points: {} without Live Detail, {} with it; inside the radius {} became {}; new or \
         moved points reach {far} from the stroke",
        off.positions.len(),
        on.positions.len(),
        inside(&before),
        inside(&on)
    );
    assert_eq!(
        off.positions.len(),
        n,
        "without Live Detail the count stays"
    );
    assert!(
        inside(&on) > inside(&before),
        "with it the count rises inside"
    );
    assert!(far <= 0.3 + 2.0 * edge, "and only there: {far}");
}

/// The Check: Pull Out with Live Detail keeps its triangle edges within twice the target. A
/// pull of 10 radii straight out of a flat grid, refining as it goes at Live Detail's default
/// size, a quarter of the radius. At 0.4 of the radius the pull falls short of 9 radii: Live
/// Detail's collapses and relaxation slide the tip off the hook's centre, and the tip then lags
/// more each dab until the brush lets go of it (decision 0049, "Measured").
#[test]
fn pull_out_with_live_detail_keeps_its_edges_within_twice_the_target() {
    let r = 0.1;
    let detail = LiveDetail::default();
    let target = match detail.target {
        Target::Relative(k) => k * r,
        Target::Constant(l) => l,
    };
    let mut s = store(&grid(48));
    let path: Vec<StrokeSample> = (0..=50)
        .map(|i| at(Vec3::new(0.0, 0.0, 10.0 * r * i as f32 / 50.0), r))
        .collect();
    let mut prm = params(None);
    prm.live_detail = Some(detail);
    run(&mut s, &preset("pull-out").preset, prm, &path);
    let m = s.to_plain();
    let tip = m.positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
    let mut longest = 0.0f32;
    for c in corners(&m) {
        for k in 0..c.len() {
            let d = Vec3::from_array(m.positions[c[k]])
                - Vec3::from_array(m.positions[c[(k + 1) % c.len()]]);
            longest = longest.max(d.len2().sqrt());
        }
    }
    eprintln!(
        "the tip rose to {tip}; {} points; the longest edge {longest} against the target {target}",
        m.positions.len()
    );
    assert!(tip > 9.0 * r, "the pull rose {tip}");
    assert!(longest <= 2.0 * target, "the longest edge is {longest}");
}
