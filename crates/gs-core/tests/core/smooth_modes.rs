//! The Smooth modifiers' Checks (catalog `brush-settings.smooth-modifiers`, decision 0049), each
//! a named test on a real mesh: smoothing along and across the stroke, a face group border kept,
//! Projection on a sphere, and Min Connected Points on an open border. The slow reference
//! (`brush_reference.rs`) covers the four older modes only; these modes have none, so each test
//! sets its mode beside a contrast the measure must tell apart.

use gs_core::groups::GroupMaker;
use gs_core::{BrushPreset, StrokeParams, StrokeSample, Vec3, View};
use gs_protocol::PlainMesh;
use gs_testkit::fixtures;
use gs_testkit::measure::stats;

use crate::brush_checks::{Height, jitter, relief};
use crate::brush_kit::{at, front, max_diff, preset, run, store};
use crate::common::{mix, ok, unit};

/// Smooth with its kernel's mode set to `mode` and the kernel keys `keys` added.
fn smooth(mode: &str, keys: &str) -> BrushPreset {
    let text = preset("smooth").text;
    let from = "mode = \"uniform\"\n";
    assert!(text.contains(from), "Smooth's file names its mode");
    let text = text.replacen(from, &format!("mode = \"{mode}\"\n{keys}\n"), 1);
    ok(BrushPreset::from_toml(&text))
}

/// `passes` strokes of `p` along `path` on `mesh`, each seen through `view`.
fn passes(
    mesh: &PlainMesh,
    p: &BrushPreset,
    view: View,
    path: &[StrokeSample],
    n: usize,
) -> PlainMesh {
    let mut s = store(mesh);
    for _ in 0..n {
        run(&mut s, p, StrokeParams::new(view), path);
    }
    s.to_plain()
}

/// A ridge 0.1 high and about two edges wide on the 48 grid, running through the origin along
/// the unit direction `(dx, dy)`, in the store's vertex order.
fn ridge(dx: f32, dy: f32) -> PlainMesh {
    relief(48, &move |x, y| {
        let d = (dy * x - dx * y) / 0.08;
        0.1 * libm::expf(-d * d)
    })
}

/// The mean height left on the ridge line along `(dx, dy)` within 0.1 of the origin, as a share
/// of the 0.1 it started at. The line runs through grid points, so they sit on it exactly.
fn kept(before: &PlainMesh, after: &PlainMesh, (dx, dy): (f32, f32)) -> f64 {
    let on: Vec<usize> = (0..before.positions.len())
        .filter(|&i| {
            let [x, y, _] = before.positions[i];
            (dy * x - dx * y).abs() < 1e-6 && x * x + y * y < 0.01
        })
        .collect();
    assert!(on.len() >= 3, "{} points on the ridge line", on.len());
    let z: f64 = on.iter().map(|&i| f64::from(after.positions[i][2])).sum();
    z / on.len() as f64 / 0.1
}

/// The Check: mode 4 smooths a diagonal ridge only along the stroke. The stroke runs along the
/// grid's diagonal, so neither direction is an edge direction: under `along_stroke` a ridge
/// lying along the stroke keeps its height and one crossing it is smoothed away, under
/// `across_stroke` (mode 5) the other way round, and uniform smoothing lowers both.
#[test]
fn along_stroke_smooths_a_diagonal_ridge_only_along_the_stroke() {
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let (lying, crossing) = ((s, s), (s, -s));
    let r = 0.25;
    let path = |z: Height| -> Vec<StrokeSample> {
        (0..=24)
            .map(|i| {
                let t = -0.4 + 0.8 * i as f32 / 24.0;
                at(Vec3::new(t, t, z(t, t)), r)
            })
            .collect()
    };
    let view = View {
        dir: Vec3::new(0.0, 0.0, -1.0),
        eye: None,
    };
    let left = |mode: &str, dir: (f32, f32)| {
        let before = ridge(dir.0, dir.1);
        let z = |x: f32, y: f32| {
            let d = (dir.1 * x - dir.0 * y) / 0.08;
            0.1 * libm::expf(-d * d)
        };
        let after = passes(&before, &smooth(mode, ""), view, &path(&z), 4);
        kept(&before, &after, dir)
    };
    let along = (left("along_stroke", lying), left("along_stroke", crossing));
    let across = (
        left("across_stroke", lying),
        left("across_stroke", crossing),
    );
    let uniform = (left("uniform", lying), left("uniform", crossing));
    eprintln!(
        "height kept (ridge along the stroke, ridge across it): along_stroke {along:?}, \
         across_stroke {across:?}, uniform {uniform:?}"
    );
    assert!(
        along.0 > 0.99,
        "along_stroke kept the lying ridge: {along:?}"
    );
    assert!(
        along.1 < 0.8,
        "along_stroke smoothed the crossing ridge: {along:?}"
    );
    assert!(
        across.1 > 0.99,
        "across_stroke kept the crossing ridge: {across:?}"
    );
    assert!(
        across.0 < 0.8,
        "across_stroke smoothed the lying ridge: {across:?}"
    );
    assert!(
        uniform.0 < 0.95 && uniform.1 < 0.95,
        "uniform lowers both: {uniform:?}"
    );
}

/// The Check: mode 6 keeps a group border within tolerance where mode 0 pulls it in. A cube with
/// one face group per side, smoothed along the edge where the +x and +y sides meet: under
/// `group_borders` the edge's points stay on the edge, under `uniform` the edge rounds, and
/// half the Weight Strength rounds it less.
#[test]
fn group_borders_keep_a_group_border_where_uniform_pulls_it_in() {
    let mut cube = store(&fixtures::cube(24));
    ok(cube.make_groups(GroupMaker::Normals { max_angle: 20.0 }));
    let before = cube.to_plain();
    let path: Vec<StrokeSample> = (0..=24)
        .map(|i| at(Vec3::new(1.0, 1.0, -0.4 + 0.8 * i as f32 / 24.0), 0.3))
        .collect();
    let view = View {
        dir: Vec3::new(-1.0, -1.0, 0.0).normalized(),
        eye: None,
    };
    let edge: Vec<usize> = (0..before.positions.len())
        .filter(|&i| {
            let [x, y, z] = before.positions[i];
            (x - 1.0).abs() < 1e-6 && (y - 1.0).abs() < 1e-6 && z.abs() < 0.25
        })
        .collect();
    assert!(edge.len() >= 5, "{} points on the edge", edge.len());
    let off = |p: &BrushPreset| {
        let after = passes(&before, p, view, &path, 3);
        let gone = |i: &usize| {
            let [x, y, _] = after.positions[*i];
            f64::from(((x - 1.0) * (x - 1.0) + (y - 1.0) * (y - 1.0)).sqrt())
        };
        let d: Vec<f64> = edge.iter().map(gone).collect();
        let most = d.iter().copied().fold(0.0, f64::max);
        let least = d.iter().copied().fold(f64::INFINITY, f64::min);
        (least, most)
    };
    let borders = off(&smooth("group_borders", ""));
    let half = off(&smooth("group_borders", "weight = 0.5"));
    let uniform = off(&smooth("uniform", ""));
    eprintln!(
        "edge points off the edge (least, most): group_borders {borders:?}, at weight 0.5 \
         {half:?}, uniform {uniform:?}"
    );
    assert!(
        borders.1 < 1e-5,
        "group_borders kept the border: {borders:?}"
    );
    assert!(
        uniform.0 > 0.01,
        "uniform pulled every edge point in: {uniform:?}"
    );
    assert!(
        half.0 > borders.1 && half.1 < uniform.0,
        "half the weight rounds it less: {half:?} against {uniform:?}"
    );
}

/// The unit sphere: the testkit's 24 cube pushed out to it, each point then slid along it by up
/// to 0.02 at random, in the store's vertex order.
fn slid_sphere() -> PlainMesh {
    let mut ball = fixtures::cube(24);
    for (i, p) in ball.positions.iter_mut().enumerate() {
        let k = 3 * i as u64;
        let slide = Vec3::new(unit(mix(k)), unit(mix(k + 1)), unit(mix(k + 2)));
        let d = Vec3::from_array(*p).normalized();
        *p = (d + slide * 0.02).normalized().to_array();
    }
    store(&ball).to_plain()
}

/// `passes` rounds of short strokes of `p`, radius 1.2, from each of the six axis directions
/// with the camera looking along it, so the whole ball is smoothed.
fn all_over(m: &PlainMesh, p: &BrushPreset, passes: usize) -> PlainMesh {
    let mut s = store(m);
    let x = Vec3::new(1.0, 0.0, 0.0);
    let (y, z) = (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
    for _ in 0..passes {
        for a in [x, y, z, x * -1.0, y * -1.0, z * -1.0] {
            let side = a.cross(Vec3::new(0.3, 0.5, 0.8)).normalized();
            let smp = [at(a, 1.2), at((a + side * 0.1).normalized(), 1.2)];
            let view = View {
                dir: a * -1.0,
                eye: None,
            };
            run(&mut s, p, StrokeParams::new(view), &smp);
        }
    }
    s.to_plain()
}

/// The Check: repeated smoothing of a sphere keeps its points on it with Projection on and
/// shrinks it with Projection off. The points are slid along the sphere first, so smoothing has
/// spacing to even out and does move them.
#[test]
fn projection_keeps_a_smoothed_sphere_on_itself_and_without_it_the_sphere_shrinks() {
    let ball = slid_sphere();
    let v0 = stats(&ball).volume;
    let off = |m: &PlainMesh| {
        let r = |p: &[f32; 3]| f64::from((Vec3::from_array(*p).len2().sqrt() - 1.0).abs());
        m.positions.iter().map(r).fold(0.0, f64::max)
    };
    let on = all_over(&ball, &smooth("uniform", "projection = true"), 4);
    let plain = all_over(&ball, &smooth("uniform", ""), 4);
    let change = |m: &PlainMesh| (stats(m).volume - v0) / v0;
    let moved = max_diff(&ball, &on);
    eprintln!(
        "off the sphere: Projection on {}, off {}; volume change on {}, off {}; Projection on \
         moved points up to {moved}",
        off(&on),
        off(&plain),
        change(&on),
        change(&plain)
    );
    assert!(moved > 0.005, "smoothing moved points by {moved}");
    assert!(
        off(&on) < 0.002,
        "Projection kept the points on the sphere: {}",
        off(&on)
    );
    assert!(change(&on).abs() < 0.001, "and the volume: {}", change(&on));
    assert!(
        change(&plain) < -0.01,
        "without it the sphere shrank: {}",
        change(&plain)
    );
}

/// The Check: at Min Connected Points 1 an open plane's border points move, at a higher value
/// they stay. A stroke along the border of a noisy plane: there a point averages its two border
/// neighbours, so at 2 it still moves and at 3 it stays to the bit while the points inside move.
/// The shipped Smooth (no `min_connected`) smooths to the bit as 1 does, so today's Smooth
/// behaves as 1.
#[test]
fn min_connected_points_one_moves_an_open_border_and_three_keeps_it() {
    let noise = jitter(0.01);
    let before = relief(32, &noise);
    let path: Vec<StrokeSample> = (0..=24)
        .map(|i| {
            let x = -0.6 + 1.2 * i as f32 / 24.0;
            at(Vec3::new(x, -1.0, noise(x, -1.0)), 0.25)
        })
        .collect();
    let border = |i: usize| {
        let [x, y, _] = before.positions[i];
        (y + 1.0).abs() < 1e-6 && x.abs() < 0.4
    };
    let bits = |p: [f32; 3]| p.map(f32::to_bits);
    let moved = |m: &PlainMesh, on: bool| {
        (0..before.positions.len())
            .filter(|&i| border(i) == on && bits(m.positions[i]) != bits(before.positions[i]))
            .count()
    };
    let view = front();
    let shipped = passes(&before, &preset("smooth").preset, view, &path, 1);
    let at_least = |n: u32| {
        passes(
            &before,
            &smooth("uniform", &format!("min_connected = {n}")),
            view,
            &path,
            1,
        )
    };
    let (one, two, three) = (at_least(1), at_least(2), at_least(3));
    let count = (0..before.positions.len()).filter(|&i| border(i)).count();
    eprintln!(
        "of {count} border points, moved: shipped {}, at 1 {}, at 2 {}, at 3 {}; inside at 3 {}",
        moved(&shipped, true),
        moved(&one, true),
        moved(&two, true),
        moved(&three, true),
        moved(&three, false)
    );
    assert_eq!(moved(&one, true), count, "at 1 every border point moved");
    assert_eq!(moved(&two, true), count, "at 2 too: a border point has two");
    assert_eq!(moved(&three, true), 0, "at 3 none did");
    assert!(moved(&three, false) > 20, "while the points inside moved");
    assert_eq!(
        shipped.positions, one.positions,
        "the shipped Smooth smooths as 1 does"
    );
}
