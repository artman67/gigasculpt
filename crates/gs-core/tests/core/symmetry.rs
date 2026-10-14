//! The symmetry feather of one dab (decision 0040) against Blender's rule written out in `f64`
//! (`calc_symmetry_feather`, `mesh_paint.cc`): one over the summed overlap of the dab's passes,
//! a pass whose center lies `d` from the dab's overlapping it by `1 - d / (2 r)`, and not at all
//! from `2 r` apart. The pass centers are worked out here, sharing only data with the code.
//!
//! Then the symmetry types (decision 0044): where Mirror, Parallel and Radial put a dab's
//! copies, which way a Parallel copy goes, and that a gesture repeats across the planes.

use std::collections::BTreeSet;

use gs_core::math::Vec3;
use gs_core::symmetry::Symmetry;
use gs_core::{NO, StrokeParams};
use gs_protocol::PlainMesh;
use gs_testkit::rng::SplitMix64;

use crate::brush_kit::{
    after, at, hidden_faces, ico, max_diff, mirror_pairs, mirror_x, params, preset, store,
};

/// Blender's feather for a dab at `p` with radius `r`, from its own flips and turns.
fn blender(p: [f64; 3], mirror: [bool; 3], radial: Option<(usize, u32)>, r: f64) -> f64 {
    let (axis, n) = radial.unwrap_or((0, 1));
    let mut overlap = 0.0;
    for mask in 0..8u32 {
        if (0..3).any(|a| mask >> a & 1 == 1 && !mirror[a]) {
            continue;
        }
        let flip: [f64; 3] = std::array::from_fn(|a| if mask >> a & 1 == 1 { -p[a] } else { p[a] });
        for k in 0..n {
            let t = std::f64::consts::TAU * f64::from(k) / f64::from(n);
            let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
            let (s, c) = (libm::sin(t), libm::cos(t));
            let mut q = flip;
            q[i] = flip[i] * c - flip[j] * s;
            q[j] = flip[i] * s + flip[j] * c;
            let d = (0..3)
                .map(|a| (q[a] - p[a]) * (q[a] - p[a]))
                .sum::<f64>()
                .sqrt();
            if d <= 2.0 * r {
                overlap += (2.0 * r - d) / (2.0 * r);
            }
        }
    }
    1.0 / overlap
}

#[test]
fn the_feather_of_a_dab_is_blenders_overlap_rule() {
    let mut rng = SplitMix64::new(0x6720);
    // Uniform in [0, 1): the top 53 of 64 random bits.
    let mut next = || (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
    for case in 0..2000 {
        let mirror = [next() < 0.6, next() < 0.4, next() < 0.4];
        let radial = (next() < 0.4).then(|| ((next() * 3.0) as usize, 2 + (next() * 6.0) as u32));
        let r = 0.02 + next();
        // Some dabs sit on a plane or the radial axis, where passes meet exactly.
        let mut p: [f64; 3] = std::array::from_fn(|_| 2.0 * next() - 1.0);
        for c in &mut p {
            if next() < 0.15 {
                *c = 0.0;
            }
        }
        let sym = Symmetry {
            mirror,
            radial,
            ..Symmetry::default()
        };
        let at = Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32);
        let got = f64::from(sym.feather_scale(at, r as f32));
        let want = blender(
            p.map(|c| f64::from(c as f32)),
            mirror,
            radial,
            f64::from(r as f32),
        );
        assert!(
            (got - want).abs() <= 1e-5 * want,
            "case {case}: {sym:?} at {p:?} r {r}: {got} not {want}"
        );
    }
}

#[test]
fn a_dab_on_the_plane_counts_once_and_far_passes_keep_their_strength() {
    let x = Symmetry {
        mirror: [true, false, false],
        ..Symmetry::default()
    };
    let on = x.feather_scale(Vec3::new(0.0, 0.4, 0.9), 0.25);
    assert_eq!(on, 0.5, "two passes at one place count as one");
    // Passes exactly twice the reach apart no longer overlap.
    let apart = x.feather_scale(Vec3::new(0.25, 0.4, 0.9), 0.25);
    assert_eq!(apart, 1.0, "passes 2 r apart keep their strength");
    let off = Symmetry {
        feather: false,
        ..x
    };
    assert_eq!(
        off.feather_scale(Vec3::new(0.0, 0.4, 0.9), 0.25),
        1.0,
        "feather off"
    );
    assert_eq!(
        Symmetry::default().feather_scale(Vec3::new(0.0, 0.0, 0.0), 0.25),
        1.0,
        "one pass, nothing to share"
    );
}

/// The plane x = 0, Parallel.
fn parallel_x() -> Symmetry {
    Symmetry {
        parallel: [true, false, false],
        ..Symmetry::default()
    }
}

fn flip_x(v: Vec3) -> Vec3 {
    Vec3::new(-v.x, v.y, v.z)
}

/// Each vertex's move from `before` to `after`.
fn moves(before: &PlainMesh, after: &PlainMesh) -> Vec<Vec3> {
    (before.positions.iter().zip(&after.positions))
        .map(|(a, b)| Vec3::from_array(*b) - Vec3::from_array(*a))
        .collect()
}

/// The vertices that moved from `before` to `after` gather at the `centers`: some moved within
/// `reach` of each, and none farther than `reach` from every one.
fn copies_at(before: &PlainMesh, after: &PlainMesh, centers: &[Vec3], reach: f32) {
    let moved: Vec<Vec3> = (before.positions.iter().zip(&after.positions))
        .filter(|(a, b)| a.map(f32::to_bits) != b.map(f32::to_bits))
        .map(|(a, _)| Vec3::from_array(*a))
        .collect();
    let near = |p: Vec3, c: Vec3| (p - c).len2() <= reach * reach;
    for (i, &c) in centers.iter().enumerate() {
        let seen = moved.iter().any(|&p| near(p, c));
        assert!(seen, "nothing moved at copy {i}, {c:?}");
    }
    for &p in &moved {
        let inside = centers.iter().any(|&c| near(p, c));
        assert!(inside, "{p:?} moved away from every copy");
    }
}

/// `m` in the order a stroke hands it back, so vertex `i` before is vertex `i` after.
fn stored(m: &PlainMesh) -> PlainMesh {
    store(m).to_plain()
}

/// Two unit spheres, the ears, centered at x = -2 and x = 2: mirror images bit for bit.
fn ears() -> PlainMesh {
    let one = ico(3);
    let n = one.positions.len() as u32;
    let mut m = PlainMesh::default();
    for dx in [-2.0f32, 2.0] {
        let moved = one.positions.iter().map(|&[x, y, z]| [dx + x, y, z]);
        m.positions.extend(moved);
    }
    for k in [0, n] {
        let shifted = |f: [u32; 4]| f.map(|v| if v == NO { NO } else { v + k });
        m.faces.extend(one.faces.iter().map(|&f| shifted(f)));
    }
    stored(&m)
}

#[test]
fn with_x_on_one_dab_moves_two_places_mirror_images_of_each_other() {
    let mesh = stored(&ico(4));
    let p = Vec3::new(0.5, 0.3, 0.8).normalized();
    let draw = preset("draw").preset;
    let out = after(&mesh, &draw, params(Some(mirror_x())), &[at(p, 0.2)]);
    copies_at(&mesh, &out, &[p, flip_x(p)], 0.25);
    let (d, pairs) = (moves(&mesh, &out), mirror_pairs(&mesh));
    let worst = (0..d.len())
        .map(|i| (d[pairs[i]] - flip_x(d[i])).len2().sqrt())
        .fold(0.0, f32::max);
    assert!(
        worst < 1e-5,
        "the two moves mirror each other: off by {worst}"
    );
}

#[test]
fn radial_8_around_y_moves_8_places() {
    let mesh = stored(&ico(4));
    let p = Vec3::new(0.8, 0.3, 0.5).normalized();
    let sym = Symmetry {
        radial: Some((1, 8)),
        ..Symmetry::default()
    };
    let draw = preset("draw").preset;
    let out = after(&mesh, &draw, params(Some(sym)), &[at(p, 0.15)]);
    let turned = |k: u32| {
        let t = std::f32::consts::TAU * k as f32 / 8.0;
        let (s, c) = (libm::sinf(t), libm::cosf(t));
        Vec3::new(p.x * c + p.z * s, p.y, p.z * c - p.x * s)
    };
    let centers: Vec<Vec3> = (0..8).map(turned).collect();
    copies_at(&mesh, &out, &centers, 0.2);
}

/// The Check's ears: dragging the left ear's front 1 unit toward -X moves the right ear's
/// front toward +X in Mirror and toward -X in Parallel, the same move with its X turned.
#[test]
fn move_drags_the_other_ear_toward_plus_x_in_mirror_and_minus_x_in_parallel() {
    let mesh = ears();
    let near = |p: Vec3| {
        let d = |i: &usize| (Vec3::from_array(mesh.positions[*i]) - p).len2();
        (0..mesh.positions.len()).min_by(|i, j| d(i).total_cmp(&d(j)))
    };
    let left = near(Vec3::new(-2.0, 0.0, 1.0)).unwrap_or_default();
    let right = mirror_pairs(&mesh)[left];
    let grab = Vec3::from_array(mesh.positions[left]);
    let drag: Vec<_> = (0..=4)
        .map(|i| at(grab + Vec3::new(-0.25 * i as f32, 0.0, 0.0), 0.5))
        .collect();
    let mv = preset("move").preset;
    let moved = |sym: Symmetry| moves(&mesh, &after(&mesh, &mv, params(Some(sym)), &drag));
    let (m, p) = (moved(mirror_x()), moved(parallel_x()));
    let (ml, pl) = (m[left], p[left]);
    assert!(
        ml.x < -0.5 && ml == pl,
        "the left ear goes toward -X: {ml:?}, {pl:?}"
    );
    assert!(
        m[right].x > 0.5,
        "Mirror: the right ear toward +X, {:?}",
        m[right]
    );
    assert!(
        p[right].x < -0.5,
        "Parallel: the right ear toward -X, {:?}",
        p[right]
    );
    let off = (p[right] - flip_x(m[right])).len2().sqrt();
    assert!(off < 1e-5, "the same move, its X turned: off by {off}");
}

/// A dab with no direction reads only where it lands, which both types place alike: one Clay
/// Build dab, whose square reads the same turned and flipped, is the same within float error.
#[test]
fn one_clay_build_dab_is_the_same_in_mirror_and_parallel() {
    let mesh = stored(&ico(4));
    let clay = preset("clay-build").preset;
    let dab = [at(Vec3::new(0.45, 0.3, 0.84).normalized(), 0.3)];
    let with = |sym: Option<Symmetry>| after(&mesh, &clay, params(sym), &dab);
    let (alone, m, p) = (with(None), with(Some(mirror_x())), with(Some(parallel_x())));
    assert!(max_diff(&m, &alone) > 1e-3, "the copy moved the other side");
    let off = max_diff(&m, &p);
    assert!(off < 1e-5, "Mirror and Parallel differ by {off}");
}

/// The hidden faces of `m`, each by its corners' positions sorted, mirrored across x = 0 if
/// `flip`.
fn hidden_corners(m: &PlainMesh, flip: bool) -> BTreeSet<Vec<[u32; 3]>> {
    let at = |v: u32| {
        let [x, y, z] = m.positions[v as usize];
        [if flip { 0.0 - x } else { x }, y, z].map(f32::to_bits)
    };
    (m.faces.iter().zip(hidden_faces(m)))
        .filter(|(_, hidden)| *hidden)
        .map(|(f, _)| {
            let mut k: Vec<[u32; 3]> = f.iter().filter(|&&v| v != NO).map(|&v| at(v)).collect();
            k.sort_unstable();
            k
        })
        .collect()
}

/// Alt and Select Rect over the top of the left ear hide it; with X on, Mirror or Parallel, the
/// right ear's top hides too, its mirror image.
#[test]
fn with_x_on_a_hide_rectangle_over_one_ear_hides_both() {
    let mesh = ears();
    let rect = [(-3.4, 0.2), (-0.6, 1.4)];
    let rect = rect.map(|(x, y)| at(Vec3::new(x, y, 2.0), 0.1));
    let select = preset("select-rect").preset;
    let hide = |sym: Option<Symmetry>| {
        let prm = StrokeParams {
            invert: true,
            ..params(sym)
        };
        after(&mesh, &select, prm, &rect)
    };
    // A face is on the left ear if its first corner is.
    let sides = |m: &PlainMesh| {
        let hidden = (m.faces.iter().zip(hidden_faces(m))).filter(|(_, h)| *h);
        let left = |f: &[u32; 4]| m.positions[f[0] as usize][0] < 0.0;
        hidden.fold(
            (0, 0),
            |(l, r), (f, _)| if left(f) { (l + 1, r) } else { (l, r + 1) },
        )
    };
    let (left, right) = sides(&hide(None));
    assert!(
        left > 20 && right == 0,
        "alone, the left ear: {left} and {right}"
    );
    for sym in [mirror_x(), parallel_x()] {
        let out = hide(Some(sym));
        assert_eq!(sides(&out), (left, left), "{sym:?}: both ears");
        let (hidden, mirrored) = (hidden_corners(&out, false), hidden_corners(&out, true));
        assert!(hidden == mirrored, "{sym:?}: mirror images");
    }
}
