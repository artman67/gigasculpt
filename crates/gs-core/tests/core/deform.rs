//! Deformers as `transform.apply` operations (decision 0041, amended 2026-10-12): each one's
//! meaning on a known shape against a reference worked out here in `f64`; the weight rule (a
//! weight scales a turning operation's angle, a linear one's argument, and the change of clip
//! and spherize); the bend's precision at every curvature and its exact zero; small turns
//! moving points little; the default box and radius measured the same at 1 and 24 threads; the
//! shared `axes` and `gradient`; and every operation on a part's own points landing where it
//! lands on the part baked. Differential checks and determinism across thread counts
//! (ARCHITECTURE 10.1).

use std::collections::BTreeMap;
use std::f32::consts::{FRAC_PI_2, PI};

use gs_core::deform::{Deform, Gradient, Scope};
use gs_core::symmetry::Symmetry;
use gs_core::transform::{Distance, Frame, Op, RIGID, Similarity, Transform, Weights};
use gs_core::{Aabb, MeshStore, Quat, Vec3};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};

use crate::common::{ok, small_nodes, sphere};

/// A flat strip along x from 0 to `length`: three rows of points at y = -h, 0 and h, `n` quads
/// long, so its middle row is its centre line. Point `3 i + r` is column `i`, row `r`.
pub(crate) fn strip(n: u32, length: f32, h: f32) -> PlainMesh {
    let at = |i: u32, r: u32| i * 3 + r;
    let positions = (0..=n)
        .flat_map(|i| {
            (0..3).map(move |r| [length * i as f32 / n as f32, (r as f32 - 1.0) * h, 0.0])
        })
        .collect();
    let mut faces = Vec::new();
    for i in 0..n {
        for r in 0..2 {
            faces.push([at(i, r), at(i + 1, r), at(i + 1, r + 1), at(i, r + 1)]);
        }
    }
    PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    }
}

/// `op` rigid about a frame at `origin` with `axes`, the mask holding points back, no symmetry
/// and the shared arguments at their defaults.
pub(crate) fn about(origin: [f32; 3], axes: Quat, op: Op) -> Transform {
    Transform {
        frame: Frame {
            origin: Vec3::from_array(origin),
            axes,
        },
        op,
        weights: Weights {
            mask: true,
            falloff: RIGID,
            reach: 0.0,
            distance: Distance::Straight,
        },
        symmetry: Symmetry::default(),
        scope: Scope::default(),
    }
}

/// `d` about the scene's own frame.
pub(crate) fn deform(d: Deform) -> Transform {
    about([0.0; 3], Quat::IDENTITY, Op::Deform(d))
}

/// `mesh`'s points after `t`, in the mesh's own order.
pub(crate) fn after(mesh: &PlainMesh, t: &Transform) -> Vec<[f32; 3]> {
    let (mut store, placed) = ok(MeshStore::import(mesh.clone(), small_nodes()));
    store.transform(t);
    let out = store.to_plain();
    (placed.vertices.iter())
        .map(|&i| out.positions[i as usize])
        .collect()
}

pub(crate) fn wide(p: [f32; 3]) -> [f64; 3] {
    p.map(f64::from)
}

pub(crate) fn gap(a: [f32; 3], b: [f64; 3]) -> f64 {
    let a = wide(a);
    (a.iter().zip(&b))
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f64>()
        .sqrt()
}

/// The largest gap between two lists of points, point by point.
pub(crate) fn worst(got: &[[f32; 3]], want: &[[f64; 3]]) -> f64 {
    (got.iter().zip(want))
        .map(|(&g, &w)| gap(g, w))
        .fold(0.0, f64::max)
}

/// `mesh` with every vertex masked `m`.
pub(crate) fn masked(mesh: &PlainMesh, m: impl Fn([f32; 3]) -> f32) -> PlainMesh {
    let mut mesh = mesh.clone();
    let values = mesh.positions.iter().map(|&p| m(p)).collect();
    let mask = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(values),
    };
    mesh.attributes.insert("mask".into(), mask);
    mesh
}

/// One of each operation, at an amount that changes every part of a sphere about the origin.
fn every_op() -> Vec<Deform> {
    vec![
        Deform::Bend {
            axis: 1,
            toward: 0,
            angle: 1.1,
        },
        Deform::Fold {
            axis: 0,
            toward: 2,
            angle: 0.7,
        },
        Deform::Twist {
            axis: 2,
            angle: 2.0,
        },
        Deform::Taper {
            axis: 1,
            amount: 0.6,
            exponent: 1.5,
        },
        Deform::Shear {
            axis: 2,
            toward: 1,
            amount: 0.4,
            smooth: true,
        },
        Deform::Clip {
            axis: 0,
            depth: 0.5,
        },
        Deform::Spherize {
            amount: 0.8,
            radius: Some(0.7),
        },
        Deform::Noise {
            amount: 0.05,
            scale: 0.4,
            seed: 11,
        },
    ]
}

/// The bend of point `p` by `angle` about the scene's frame, its arc over x from 0 to `length`
/// turning toward y, straight beyond: the textbook arc, in `f64`.
fn bent(p: [f32; 3], angle: f32, length: f32) -> [f64; 3] {
    let ([s, y, z], a, l) = (wide(p), f64::from(angle), f64::from(length));
    if a == 0.0 {
        return [s, y, z];
    }
    let r = l / a;
    let x = s.clamp(0.0, l);
    let (e, t) = (s - x, x / r);
    let (sin, cos) = (libm::sin(t), libm::cos(t));
    [
        r * sin - y * sin + e * cos,
        r * (1.0 - cos) + y * cos + e * sin,
        z,
    ]
}

#[test]
fn a_bend_of_90_degrees_turns_a_bars_end_a_quarter_and_keeps_its_centre_line() {
    let (n, length, h) = (64, 2.0, 0.05);
    let bar = strip(n, length, h);
    let bend = Deform::Bend {
        axis: 0,
        toward: 1,
        angle: FRAC_PI_2,
    };
    let got = after(&bar, &deform(bend));
    let centre: Vec<[f32; 3]> = (0..=n).map(|i| got[(i * 3 + 1) as usize]).collect();
    let arc: Vec<[f64; 3]> = (0..=n)
        .map(|i| bent(bar.positions[(i * 3 + 1) as usize], FRAC_PI_2, length))
        .collect();
    assert!(worst(&centre, &arc) < 1e-6, "{}", worst(&centre, &arc));
    // The chords of an arc of length L, cut in n, add up to L sinc(a / 2n).
    let walked: f64 = (centre.windows(2)).map(|w| gap(w[0], wide(w[1]))).sum();
    let half = f64::from(FRAC_PI_2) / f64::from(2 * n);
    let chords = f64::from(length) * libm::sin(half) / half;
    assert!((walked - chords).abs() < 1e-6, "{walked} against {chords}");
    let end = (n * 3) as usize;
    let across = [0, 1, 2].map(|k| f64::from(got[end + 2][k]) - f64::from(got[end][k]));
    let want = [-2.0 * f64::from(h), 0.0, 0.0];
    assert!(
        (0..3).all(|k| (across[k] - want[k]).abs() < 1e-6),
        "the end turned a quarter: {across:?}"
    );
}

#[test]
fn a_bend_is_the_arc_within_rounding_at_every_curvature_and_exact_at_zero() {
    let (n, length, h) = (200, 2.0, 0.1);
    let bar = strip(n, length, h);
    let box_to = |hi: f32| Aabb {
        lo: Vec3::new(0.0, -1.0, -1.0),
        hi: Vec3::new(hi, 1.0, 1.0),
    };
    for angle in [
        1e-9,
        1e-6,
        1e-3,
        0.1,
        1.0,
        FRAC_PI_2,
        PI,
        2.0 * PI,
        4.0 * PI,
        -2.5,
    ] {
        // Over the whole bar, and over its first half with the rest going on straight.
        for limit in [length, length / 2.0] {
            let mut t = deform(Deform::Bend {
                axis: 0,
                toward: 1,
                angle,
            });
            t.scope.bounds = Some(box_to(limit));
            let got = after(&bar, &t);
            let want: Vec<[f64; 3]> = (bar.positions.iter())
                .map(|&p| bent(p, angle, limit))
                .collect();
            let off = worst(&got, &want) / f64::from(length);
            assert!(
                off < 2.5e-7,
                "angle {angle} over {limit}: off by {off} of the length"
            );
        }
    }
    let flat = after(
        &bar,
        &deform(Deform::Bend {
            axis: 0,
            toward: 1,
            angle: 0.0,
        }),
    );
    let same =
        (flat.iter().zip(&bar.positions)).all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
    assert!(same, "a bend of 0 changes no bit");
}

#[test]
fn every_operation_at_its_identity_changes_no_bit() {
    let ball = sphere(10, 0.5, 7, true);
    let none = [
        Deform::Bend {
            axis: 1,
            toward: 0,
            angle: 0.0,
        },
        Deform::Fold {
            axis: 0,
            toward: 2,
            angle: 0.0,
        },
        Deform::Twist {
            axis: 2,
            angle: 0.0,
        },
        Deform::Taper {
            axis: 1,
            amount: 0.0,
            exponent: 2.0,
        },
        Deform::Shear {
            axis: 2,
            toward: 1,
            amount: 0.0,
            smooth: false,
        },
        Deform::Clip {
            axis: 0,
            depth: 0.0,
        },
        Deform::Spherize {
            amount: 0.0,
            radius: None,
        },
        Deform::Noise {
            amount: 0.0,
            scale: 0.3,
            seed: 3,
        },
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
        let got = after(&ball, &t);
        let same = (got.iter().zip(&ball.positions))
            .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
        assert!(same, "{:?} changed a bit", t.op);
    }
}

#[test]
fn a_twist_turns_the_far_end_a_quarter_and_the_middle_an_eighth() {
    let (n, length, h) = (8, 2.0, 0.3);
    let bar = strip(n, length, h);
    let got = after(
        &bar,
        &deform(Deform::Twist {
            axis: 0,
            angle: FRAC_PI_2,
        }),
    );
    let want: Vec<[f64; 3]> = (bar.positions.iter())
        .map(|&p| {
            let [x, y, z] = wide(p);
            let t = f64::from(FRAC_PI_2) * x / f64::from(length);
            [
                x,
                y * libm::cos(t) - z * libm::sin(t),
                y * libm::sin(t) + z * libm::cos(t),
            ]
        })
        .collect();
    assert!(worst(&got, &want) < 1e-6, "{}", worst(&got, &want));
    let top = (n * 3 + 2) as usize;
    assert!(
        gap(got[top], [2.0, 0.0, 0.3]) < 1e-6,
        "the far end turned a quarter: {:?}",
        got[top]
    );
    assert_eq!(got[2], bar.positions[2], "the base stays");
}

#[test]
fn a_taper_halves_the_far_end_and_widens_with_a_negative_amount() {
    let (n, length, h) = (8, 2.0, 0.4);
    let bar = strip(n, length, h);
    for (amount, exponent) in [(0.5, 1.0), (-0.5, 1.0), (0.5, 2.0)] {
        let got = after(
            &bar,
            &deform(Deform::Taper {
                axis: 0,
                amount,
                exponent,
            }),
        );
        let want: Vec<[f64; 3]> = (bar.positions.iter())
            .map(|&p| {
                let [x, y, z] = wide(p);
                let u = x / f64::from(length);
                let f = 1.0 - f64::from(amount) * libm::pow(u, f64::from(exponent));
                [x, y * f, z * f]
            })
            .collect();
        assert!(
            worst(&got, &want) < 1e-6,
            "{amount} {exponent}: {}",
            worst(&got, &want)
        );
    }
}

#[test]
fn a_shear_moves_the_top_its_amount_the_bottom_not_at_all_and_the_rest_in_proportion() {
    let (n, length, h) = (8, 2.0, 0.4);
    let bar = strip(n, length, h);
    for smooth in [false, true] {
        let got = after(
            &bar,
            &deform(Deform::Shear {
                axis: 0,
                toward: 1,
                amount: 0.3,
                smooth,
            }),
        );
        let want: Vec<[f64; 3]> = (bar.positions.iter())
            .map(|&p| {
                let [x, y, z] = wide(p);
                let u = x / f64::from(length);
                let g = if smooth { u * u * (3.0 - 2.0 * u) } else { u };
                [x, y + 0.3 * g, z]
            })
            .collect();
        assert!(
            worst(&got, &want) < 1e-6,
            "smooth {smooth}: {}",
            worst(&got, &want)
        );
    }
}

#[test]
fn a_fold_turns_each_side_rigidly_by_half_the_angle() {
    let (n, length, h) = (8, 2.0, 0.1);
    let bar = strip(n, length, h);
    let fold = Deform::Fold {
        axis: 0,
        toward: 1,
        angle: FRAC_PI_2,
    };
    let got = after(
        &bar,
        &about([1.0, 0.0, 0.0], Quat::IDENTITY, Op::Deform(fold)),
    );
    let want: Vec<[f64; 3]> = (bar.positions.iter())
        .map(|&p| {
            let (d, y) = (f64::from(p[0]) - 1.0, f64::from(p[1]));
            // Each side turns an eighth about the line through the origin along z, rising.
            let t = std::f64::consts::FRAC_PI_4 * d.signum() * f64::from(u8::from(d != 0.0));
            let (sin, cos) = (libm::sin(t), libm::cos(t));
            [1.0 + d * cos - y * sin, d * sin + y * cos, 0.0]
        })
        .collect();
    assert!(worst(&got, &want) < 1e-6, "{}", worst(&got, &want));
}

#[test]
fn a_clip_puts_every_point_past_its_plane_onto_it_and_keeps_every_face() {
    let ball = sphere(12, 0.5, 5, false);
    let top = (ball.positions.iter()).fold(f32::NEG_INFINITY, |m, p| m.max(p[2]));
    let (mut store, placed) = ok(MeshStore::import(ball.clone(), small_nodes()));
    let faces = store.to_plain().faces;
    store.transform(&deform(Deform::Clip {
        axis: 2,
        depth: 0.4,
    }));
    let out = store.to_plain();
    assert_eq!(out.faces, faces, "every face kept");
    let plane = f64::from(top) - 0.4;
    for (k, &i) in placed.vertices.iter().enumerate() {
        let (was, now) = (ball.positions[k], out.positions[i as usize]);
        if f64::from(was[2]) > plane {
            assert!(
                (f64::from(now[2]) - plane).abs() < 1e-6,
                "{was:?} went to {now:?}"
            );
            assert_eq!(&now[..2], &was[..2], "only along the axis");
        } else {
            assert_eq!(
                now.map(f32::to_bits),
                was.map(f32::to_bits),
                "below the plane stays"
            );
        }
    }
}

#[test]
fn a_full_spherize_puts_every_point_at_the_radius() {
    let cube = gs_testkit::fixtures::cube(6);
    let got = after(
        &cube,
        &deform(Deform::Spherize {
            amount: 1.0,
            radius: Some(0.8),
        }),
    );
    for p in &got {
        assert!((gap(*p, [0.0; 3]) - 0.8).abs() < 1e-6, "{p:?}");
    }
    let (store, _) = ok(MeshStore::import(cube.clone(), small_nodes()));
    let t = store.measured(&deform(Deform::Spherize {
        amount: 1.0,
        radius: None,
    }));
    let mean = (cube.positions.iter())
        .map(|&p| gap(p, [0.0; 3]))
        .sum::<f64>()
        / cube.positions.len() as f64;
    let Op::Deform(Deform::Spherize {
        radius: Some(r), ..
    }) = t.op
    else {
        panic!("no radius measured: {t:?}");
    };
    assert!(
        (f64::from(r) - mean).abs() < 1e-6,
        "{r} against the mean distance {mean}"
    );
}

/// `mesh` with its vertices in reverse order.
fn reversed(mesh: &PlainMesh) -> PlainMesh {
    let n = mesh.positions.len() as u32;
    let positions = mesh.positions.iter().rev().copied().collect();
    let faces = (mesh.faces.iter())
        .map(|f| f.map(|v| if v == gs_core::NO { v } else { n - 1 - v }))
        .collect();
    PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    }
}

#[test]
fn noise_follows_the_points_not_their_order_and_stays_within_its_amount() {
    let ball = sphere(12, 0.5, 9, false);
    let noise = |seed| {
        deform(Deform::Noise {
            amount: 0.05,
            scale: 0.3,
            seed,
        })
    };
    let moved = |mesh: &PlainMesh, seed| -> BTreeMap<[u32; 3], [u32; 3]> {
        let got = after(mesh, &noise(seed));
        (mesh.positions.iter().zip(got))
            .map(|(p, q)| (p.map(f32::to_bits), q.map(f32::to_bits)))
            .collect()
    };
    let first = moved(&ball, 4);
    assert_eq!(
        first,
        moved(&reversed(&ball), 4),
        "the same after a reorder"
    );
    assert_ne!(first, moved(&ball, 5), "another seed, other noise");
    for (p, q) in &first {
        let (p, q) = (p.map(f32::from_bits), q.map(f32::from_bits));
        assert!(
            (0..3).all(|k| (q[k] - p[k]).abs() <= 0.05 + 1e-6),
            "{p:?} went to {q:?}"
        );
    }
    assert!(first.iter().any(|(p, q)| p != q), "it moved something");
}

/// `d` with its amount halved: what a weight of one half must do to it.
fn halved(d: Deform) -> Deform {
    match d {
        Deform::Bend {
            axis,
            toward,
            angle,
        } => Deform::Bend {
            axis,
            toward,
            angle: angle / 2.0,
        },
        Deform::Fold {
            axis,
            toward,
            angle,
        } => Deform::Fold {
            axis,
            toward,
            angle: angle / 2.0,
        },
        Deform::Twist { axis, angle } => Deform::Twist {
            axis,
            angle: angle / 2.0,
        },
        Deform::Taper {
            axis,
            amount,
            exponent,
        } => Deform::Taper {
            axis,
            amount: amount / 2.0,
            exponent,
        },
        Deform::Shear {
            axis,
            toward,
            amount,
            smooth,
        } => Deform::Shear {
            axis,
            toward,
            amount: amount / 2.0,
            smooth,
        },
        Deform::Spherize { amount, radius } => Deform::Spherize {
            amount: amount / 2.0,
            radius,
        },
        Deform::Noise {
            amount,
            scale,
            seed,
        } => Deform::Noise {
            amount: amount / 2.0,
            scale,
            seed,
        },
        // Clip's change is halved instead; the cage and the curve have their own tests.
        other => other,
    }
}

#[test]
fn a_weight_scales_the_angle_or_the_amount_and_clip_s_change() {
    let ball = sphere(10, 0.5, 3, false);
    let half = masked(&ball, |_| 0.5);
    let frame = |d| about([0.05, -0.3, 0.1], Quat::IDENTITY, Op::Deform(d));
    for d in every_op() {
        let got = after(&half, &frame(d.clone()));
        if let Deform::Clip { .. } = d {
            let full = after(&ball, &frame(d.clone()));
            let want: Vec<[f64; 3]> = (ball.positions.iter().zip(&full))
                .map(|(&p, &q)| {
                    let (p, q) = (wide(p), wide(q));
                    [0, 1, 2].map(|k| p[k] + 0.5 * (q[k] - p[k]))
                })
                .collect();
            assert!(
                worst(&got, &want) < 1e-6,
                "clip moves half way: {}",
                worst(&got, &want)
            );
            continue;
        }
        let want = after(&ball, &frame(halved(d.clone())));
        let same = (got.iter().zip(&want)).all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits));
        assert!(
            same,
            "{d:?} at weight 1/2 is {:?} at weight 1",
            halved(d.clone())
        );
    }
}

#[test]
fn a_twist_under_a_falloff_turns_each_point_part_way_keeping_its_distance_from_the_axis() {
    let (n, length, h) = (32, 2.0, 0.3);
    let bar = strip(n, length, h);
    let mut t = deform(Deform::Twist { axis: 0, angle: PI });
    t.weights.falloff = 0.0;
    t.weights.reach = 1.5;
    let got = after(&bar, &t);
    let (mut partial, mut still) = (0, 0);
    for (&p, &q) in bar.positions.iter().zip(&got) {
        let r = libm::hypot(f64::from(q[1]), f64::from(q[2]));
        assert!(
            (r - f64::from(p[1].abs())).abs() < 1e-6,
            "{p:?} went to {q:?}"
        );
        let turned = libm::atan2(f64::from(q[2]), f64::from(q[1])).abs();
        let full = f64::from(PI) * f64::from(p[0]) / f64::from(length);
        if p[1] > 0.0 && turned > 1e-6 && turned < full - 1e-3 {
            partial += 1;
        }
        if p[0] > 1.5 {
            still += usize::from(p.map(f32::to_bits) == q.map(f32::to_bits));
        }
    }
    assert!(
        partial > 4,
        "points within the reach turn part of their angle: {partial}"
    );
    assert!(still >= 3 * 7, "points past the reach stay: {still}");
}

#[test]
fn small_turns_move_points_little() {
    let ball = sphere(10, 0.5, 13, false);
    let turns = [
        Deform::Bend {
            axis: 1,
            toward: 2,
            angle: 1e-6,
        },
        Deform::Fold {
            axis: 2,
            toward: 0,
            angle: 1e-6,
        },
        Deform::Twist {
            axis: 0,
            angle: 1e-6,
        },
    ];
    let size = (ball.positions.iter())
        .map(|&p| gap(p, [0.0; 3]))
        .fold(0.0, f64::max);
    for d in turns {
        let got = after(
            &ball,
            &about([0.2, 0.1, -0.1], Quat::IDENTITY, Op::Deform(d.clone())),
        );
        let start: Vec<[f64; 3]> = ball.positions.iter().map(|&p| wide(p)).collect();
        let moved = worst(&got, &start) / size;
        assert!(
            moved < 1.2e-6,
            "{d:?} moved a point {moved} of the part's size"
        );
    }
}

/// A turned frame off the origin, and a lumpy sphere with everything at x above 0.3 masked.
fn half_masked() -> (PlainMesh, Transform) {
    let ball = masked(&sphere(24, 0.5, 21, false), |p| {
        if p[0] > 0.3 { 1.0 } else { 0.0 }
    });
    let axes = Quat::about(Vec3::new(0.3, -1.0, 0.5).normalized(), 0.9);
    let t = about(
        [0.1, 0.2, -0.05],
        axes,
        Op::Deform(Deform::Spherize {
            amount: 0.5,
            radius: None,
        }),
    );
    (ball, t)
}

#[test]
fn the_default_box_and_radius_are_the_points_that_weigh_and_the_same_on_any_thread_count() {
    let (ball, mut t) = half_masked();
    t.scope.gradient = Some(Gradient {
        axis: 0,
        from: 0.0,
        to: 1.0,
    });
    let (store, _) = ok(MeshStore::import(ball.clone(), small_nodes()));
    let measured = |threads| gs_par::with_threads(threads, || store.measured(&t));
    let one = measured(1);
    assert_eq!(
        format!("{one:?}"),
        format!("{:?}", measured(24)),
        "the same bits on 24 threads"
    );
    let (x, y, z) = t.frame.axes.axes();
    let axes = [x, y, z].map(|a| wide(a.to_array()));
    let o = wide(t.frame.origin.to_array());
    let (mut lo, mut hi, mut sum, mut count) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], 0.0, 0);
    for p in ball.positions.iter().filter(|p| p[0] <= 0.3) {
        let v = [0, 1, 2].map(|k| f64::from(p[k]) - o[k]);
        let c = axes.map(|a| a[0] * v[0] + a[1] * v[1] + a[2] * v[2]);
        for k in 0..3 {
            lo[k] = lo[k].min(c[k]);
            hi[k] = hi[k].max(c[k]);
        }
        sum += (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        count += 1;
    }
    let Some(b) = one.scope.bounds else {
        panic!("no box measured: {one:?}");
    };
    let (blo, bhi) = (wide(b.lo.to_array()), wide(b.hi.to_array()));
    assert!(
        (0..3).all(|k| (blo[k] - lo[k]).abs() < 1e-6 && (bhi[k] - hi[k]).abs() < 1e-6),
        "{b:?} against {lo:?} {hi:?}"
    );
    let Op::Deform(Deform::Spherize {
        radius: Some(r), ..
    }) = one.op
    else {
        panic!("no radius measured: {one:?}");
    };
    let mean = sum / f64::from(count);
    assert!((f64::from(r) - mean).abs() < 1e-6, "{r} against {mean}");
}

#[test]
fn the_shared_axes_and_gradient_shape_any_operation() {
    let (n, length, h) = (8, 2.0, 0.4);
    let bar = strip(n, length, h);
    let mut t = about([0.0; 3], Quat::IDENTITY, Op::Move(Vec3::new(0.3, 0.2, 0.1)));
    t.scope.axes = [true, false, false];
    let got = after(&bar, &t);
    let want: Vec<[f64; 3]> = (bar.positions.iter())
        .map(|&p| [f64::from(p[0] + 0.3), f64::from(p[1]), f64::from(p[2])])
        .collect();
    assert!(worst(&got, &want) < 1e-7, "only x: {}", worst(&got, &want));
    let mut t = about([0.0; 3], Quat::IDENTITY, Op::Move(Vec3::new(0.0, 0.0, 0.2)));
    t.scope.gradient = Some(Gradient {
        axis: 0,
        from: 0.25,
        to: 0.75,
    });
    let got = after(&bar, &t);
    let want: Vec<[f64; 3]> = (bar.positions.iter())
        .map(|&p| {
            let f = f64::from(p[0]) / f64::from(length);
            let ramp = 1.0 - ((f - 0.25) / 0.5).clamp(0.0, 1.0);
            [f64::from(p[0]), f64::from(p[1]), 0.2 * ramp]
        })
        .collect();
    assert!(
        worst(&got, &want) < 1e-7,
        "a ramp along x: {}",
        worst(&got, &want)
    );
}

#[test]
fn every_operation_on_a_parts_own_points_lands_where_it_lands_on_the_part_baked() {
    let ball = sphere(10, 0.5, 17, false);
    let place = Similarity {
        translation: Vec3::new(0.3, -0.2, 0.5),
        rotation: Quat::about(Vec3::new(1.0, 2.0, 3.0).normalized(), 0.7),
        scale: 1.7,
    };
    let own = PlainMesh {
        positions: (ball.positions.iter())
            .map(|&p| place.part_point(Vec3::from_array(p)).to_array())
            .collect(),
        ..ball.clone()
    };
    let axes = Quat::about(Vec3::new(-1.0, 0.5, 0.2).normalized(), 0.3);
    let size = (ball.positions.iter())
        .map(|&p| gap(p, [0.0; 3]))
        .fold(0.0, f64::max);
    let mut calls: Vec<Transform> = (every_op().into_iter())
        .map(|d| about([0.1, 0.05, -0.2], axes, Op::Deform(d)))
        .collect();
    let mut shaped = about([0.1, 0.05, -0.2], axes, Op::Move(Vec3::new(0.2, -0.1, 0.3)));
    shaped.scope = Scope {
        axes: [true, false, true],
        gradient: Some(Gradient {
            axis: 2,
            from: 0.9,
            to: 0.1,
        }),
        bounds: Some(Aabb {
            lo: Vec3::new(-0.5, -0.5, -0.6),
            hi: Vec3::new(0.5, 0.5, 0.4),
        }),
    };
    calls.push(shaped);
    for t in calls {
        let baked = after(&ball, &t);
        let placed: Vec<[f64; 3]> = (after(&own, &place.part_transform(&t)).into_iter())
            .map(|q| wide(place.point(Vec3::from_array(q)).to_array()))
            .collect();
        let off = worst(&baked, &placed) / size;
        assert!(off < 2e-6, "{:?} off by {off} of the size", t.op);
    }
}
