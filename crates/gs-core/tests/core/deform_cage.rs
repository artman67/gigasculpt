//! The cage, `transform.apply`'s `lattice` (decision 0041, amended 2026-10-12, step DF3): a
//! soft cage moves the points within two cells of a moved point as the padded cubic B-spline
//! does, a hard one the points within one cell trilinearly, and neither moves any other point;
//! a moved corner of a 2 by 2 by 2 hard cage moves the box's corner exactly; a weight halves
//! the offsets to the bit; and both new operations change no bit at their identity, measure
//! their box, and land on the same bits on 1 and 24 threads. Differential checks against
//! references worked out here in `f64`, and determinism across thread counts (ARCHITECTURE
//! 10.1).

use gs_core::deform::{Curve, Deform, Gradient, Lattice, Offset};
use gs_core::transform::{Op, Transform};
use gs_core::{Aabb, MeshStore, Quat, Vec3};
use gs_protocol::PlainMesh;

use crate::common::{ok, small_nodes, sphere};
use crate::deform::{about, after, deform, gap, masked, wide, worst};

/// A flat sheet across x and y from -`half` to `half` at height `z`, `n` quads a side.
fn sheet(n: u32, half: f32, z: f32) -> PlainMesh {
    let at = |i: u32, j: u32| i * (n + 1) + j;
    let step = |i: u32| -half + 2.0 * half * i as f32 / n as f32;
    let positions = (0..=n)
        .flat_map(|i| (0..=n).map(move |j| [step(i), step(j), z]))
        .collect();
    let mut faces = Vec::new();
    for i in 0..n {
        for j in 0..n {
            faces.push([at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    }
}

/// A cage of `n` points a side with point `at` moved `by`.
fn cage(soft: bool, n: u32, at: [u32; 3], by: [f32; 3]) -> Deform {
    Deform::Lattice(Lattice {
        soft,
        size: [n; 3],
        offsets: vec![Offset { at, by }],
    })
}

/// `d` about the scene's frame over the box from -2 to 2 on every axis: cells of 1 for a cage
/// of 5 points a side.
fn boxed(d: Deform) -> Transform {
    let mut t = deform(d);
    t.scope.bounds = Some(Aabb {
        lo: Vec3::new(-2.0, -2.0, -2.0),
        hi: Vec3::new(2.0, 2.0, 2.0),
    });
    t
}

/// Where coordinate `s` lies in a cage of 5 points over that box, in cells from the low face,
/// clamped into the box.
fn place(s: f32) -> f64 {
    f64::from(s).clamp(-2.0, 2.0) + 2.0
}

/// The centred cubic B-spline: the weight of a cage point `x` cells away.
fn spline(x: f64) -> f64 {
    let x = x.abs();
    if x < 1.0 {
        (4.0 - 6.0 * x * x + 3.0 * x * x * x) / 6.0
    } else if x < 2.0 {
        (2.0 - x) * (2.0 - x) * (2.0 - x) / 6.0
    } else {
        0.0
    }
}

/// The soft cage's weight of point `j` of `n` along one axis at place `f`: the B-spline, each
/// end padded with a point on the line through the last two (`2 P0 - P1`), which hands the
/// padding point's weight to those two.
fn padded(f: f64, j: u32, n: u32) -> f64 {
    let (at, last) = (f64::from(j), f64::from(n));
    let (low, high) = (spline(f + 1.0), spline(f - last));
    let mut w = spline(f - at);
    if j == 0 {
        w += 2.0 * low;
    }
    if j == 1 {
        w -= low;
    }
    if j == n - 1 {
        w += 2.0 * high;
    }
    if j == n - 2 {
        w -= high;
    }
    w
}

/// The hard cage's weight of a point `x` cells away.
fn tent(x: f64) -> f64 {
    (1.0 - x.abs()).max(0.0)
}

/// Every point of `mesh` moved by `by` times `weight` of where it was, within rounding, and
/// every point of weight 0 keeping every bit.
fn moved_by(mesh: &PlainMesh, got: &[[f32; 3]], by: [f32; 3], weight: impl Fn([f32; 3]) -> f64) {
    let mut moved = 0;
    for (&p, &q) in mesh.positions.iter().zip(got) {
        let w = weight(p);
        if w == 0.0 {
            assert_eq!(
                q.map(f32::to_bits),
                p.map(f32::to_bits),
                "{p:?} is out of the moved point's reach"
            );
            continue;
        }
        moved += 1;
        let want = [0, 1, 2].map(|k| f64::from(p[k]) + f64::from(by[k]) * w);
        assert!(gap(q, want) < 1e-6, "{p:?} went to {q:?}, not {want:?}");
    }
    assert!(moved > 100, "{moved} points moved");
}

#[test]
fn a_soft_cage_moves_the_points_within_two_cells_of_a_moved_point_smoothly_and_no_other() {
    // The sheet reaches a cell past the box, which those points meet at its faces.
    let mesh = sheet(48, 3.0, 0.25);
    let by = [0.1, -0.2, 0.3];
    // The cage's middle point, and the middle of its top face along y, where the padding
    // works. The reference is smooth to its second derivative, so matching it is smoothness.
    for at in [[2, 2, 2], [2, 4, 2]] {
        let got = after(&mesh, &boxed(cage(true, 5, at, by)));
        moved_by(&mesh, &got, by, |p| {
            (0..3).map(|k| padded(place(p[k]), at[k], 5)).product()
        });
    }
}

#[test]
fn a_hard_cage_moves_the_points_within_one_cell_of_a_moved_point_trilinearly_and_no_other() {
    let mesh = sheet(48, 3.0, 0.25);
    let by = [0.1, -0.2, 0.3];
    for at in [[2, 2, 2], [2, 4, 2]] {
        let got = after(&mesh, &boxed(cage(false, 5, at, by)));
        moved_by(&mesh, &got, by, |p| {
            (0..3)
                .map(|k| tent(place(p[k]) - f64::from(at[k])))
                .product()
        });
    }
}

#[test]
fn a_moved_corner_of_a_two_by_two_by_two_hard_cage_moves_the_boxs_corner_exactly() {
    let cube = gs_testkit::fixtures::cube(4);
    let by = [0.25, 0.5, -0.125];
    let got = after(&cube, &deform(cage(false, 2, [1, 1, 1], by)));
    let find = |c: [f32; 3]| {
        let bits = |p: [f32; 3]| p.map(f32::to_bits);
        let i = cube.positions.iter().position(|&p| bits(p) == bits(c));
        i.unwrap_or_else(|| panic!("no point at {c:?}"))
    };
    assert_eq!(
        got[find([1.0; 3])],
        [1.25, 1.5, 0.875],
        "the corner moves its offset"
    );
    assert_eq!(got[find([-1.0; 3])], [-1.0; 3], "the far corner stays");
    // On two points an axis is linear either way, so a soft cage is the same map.
    let soft = after(&cube, &deform(cage(true, 2, [1, 1, 1], by)));
    let hard: Vec<[f64; 3]> = got.iter().map(|&p| wide(p)).collect();
    assert!(worst(&soft, &hard) < 1e-6, "{}", worst(&soft, &hard));
}

#[test]
fn a_weight_scales_a_cages_offsets_to_the_bit() {
    let ball = sphere(10, 0.5, 3, false);
    let half = masked(&ball, |_| 0.5);
    let axes = Quat::about(Vec3::new(0.2, 1.0, -0.4).normalized(), 0.6);
    for soft in [true, false] {
        let lattice = |f: f32| {
            let offsets = vec![
                Offset {
                    at: [1, 2, 0],
                    by: [0.3 * f, -0.1 * f, 0.2 * f],
                },
                Offset {
                    at: [2, 0, 1],
                    by: [-0.15 * f, 0.4 * f, 0.05 * f],
                },
            ];
            let d = Deform::Lattice(Lattice {
                soft,
                size: [3, 4, 3],
                offsets,
            });
            about([0.05, -0.3, 0.1], axes, Op::Deform(d))
        };
        let got = after(&half, &lattice(1.0));
        let want = after(&ball, &lattice(0.5));
        let same = (got.iter().zip(&want)).all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
        assert!(same, "soft {soft}: weight 1/2 is every offset halved");
    }
}

#[test]
fn the_cage_and_the_curve_at_their_identity_change_no_bit() {
    let ball = sphere(10, 0.5, 7, true);
    let straight = vec![[0.0, -1.1, 0.0], [0.0, 0.2, 0.0], [0.0, 1.2, 0.0]];
    let none = [
        Deform::Lattice(Lattice {
            soft: true,
            size: [3, 3, 3],
            offsets: Vec::new(),
        }),
        Deform::Lattice(Lattice {
            soft: false,
            size: [4, 2, 3],
            offsets: vec![Offset {
                at: [1, 1, 1],
                by: [0.0; 3],
            }],
        }),
        Deform::Curve(Curve {
            axis: 1,
            points: None,
            twist: None,
            scale: None,
        }),
        // Points on the axis line from the box's low face, no twist, scale 1.
        Deform::Curve(Curve {
            axis: 1,
            points: Some(straight),
            twist: Some(vec![0.0; 3]),
            scale: Some(vec![1.0; 3]),
        }),
    ];
    for d in none {
        let mut t = about(
            [0.1, -0.2, 0.3],
            Quat::about(Vec3::new(1.0, 2.0, 2.0).normalized(), 0.4),
            Op::Deform(d),
        );
        t.scope.gradient = Some(Gradient {
            axis: 1,
            from: 0.2,
            to: 0.9,
        });
        t.scope.bounds = Some(Aabb {
            lo: Vec3::new(-1.2, -1.1, -1.3),
            hi: Vec3::new(1.1, 1.2, 1.0),
        });
        let got = after(&ball, &t);
        let same = (got.iter().zip(&ball.positions))
            .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
        assert!(same, "{:?} changed a bit", t.op);
    }
}

#[test]
fn the_cage_and_the_curve_measure_their_box_and_land_the_same_on_any_thread_count() {
    let ball = sphere(24, 0.5, 21, false);
    let origin = [0.1, 0.2, -0.05];
    let calls = [
        cage(true, 4, [1, 2, 3], [0.2, -0.1, 0.15]),
        Deform::Curve(Curve {
            axis: 2,
            points: Some(vec![[0.0, 0.0, -1.1], [0.3, 0.1, 0.0], [0.0, -0.2, 0.9]]),
            twist: Some(vec![0.0, 0.8, 1.5]),
            scale: Some(vec![1.0, 1.3, 0.7]),
        }),
    ];
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &ball.positions {
        for k in 0..3 {
            let c = f64::from(p[k]) - f64::from(origin[k]);
            lo[k] = lo[k].min(c);
            hi[k] = hi[k].max(c);
        }
    }
    for d in calls {
        let t = about(origin, Quat::IDENTITY, Op::Deform(d));
        let (store, _) = ok(MeshStore::import(ball.clone(), small_nodes()));
        let measured = |threads| gs_par::with_threads(threads, || store.measured(&t));
        let one = measured(1);
        assert_eq!(format!("{one:?}"), format!("{:?}", measured(24)));
        let Some(b) = one.scope.bounds else {
            panic!("no box measured: {one:?}");
        };
        let (blo, bhi) = (wide(b.lo.to_array()), wide(b.hi.to_array()));
        assert!(
            (0..3).all(|k| (blo[k] - lo[k]).abs() < 1e-6 && (bhi[k] - hi[k]).abs() < 1e-6),
            "{b:?} against {lo:?} {hi:?}"
        );
        let landed = |threads| gs_par::with_threads(threads, || after(&ball, &t));
        let (one, many) = (landed(1), landed(24));
        let same = (one.iter().zip(&many)).all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
        assert!(same, "{:?}: the same bits on 24 threads", t.op);
        assert!(one != ball.positions, "{:?} moved something", t.op);
    }
}
