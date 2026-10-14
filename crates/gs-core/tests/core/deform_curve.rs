//! The curve, `transform.apply`'s `curve` (decision 0041, amended 2026-10-12, step DF3): a
//! curve along a quarter circle bends a bar as a 90 degree `bend` does, past both ends too, and
//! at weight one half as a 45 degree one; a straight curve's twist and scale are `twist` and
//! `taper`; a bar on an S of four points follows it, keeping its length and width; and, for
//! the cage too, small moves move points little and a part's own points land where the part
//! baked does. Differential checks against DF2's operations and references worked out here in
//! `f64` (ARCHITECTURE 10.1).

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use gs_core::deform::{Curve, Deform, Lattice, Offset};
use gs_core::math::{dot, sub};
use gs_core::transform::{Op, Similarity, Transform};
use gs_core::{Aabb, Quat, Vec3};

use crate::common::sphere;
use crate::deform::{about, after, deform, gap, masked, strip, wide, worst};

fn curve(axis: usize, points: Option<Vec<[f32; 3]>>, twist: Option<Vec<f32>>) -> Deform {
    Deform::Curve(Curve {
        axis,
        points,
        twist,
        scale: None,
    })
}

/// `count` points evenly along a quarter circle of `radius` from x = `start` on the x axis,
/// turning toward y.
fn quarter(radius: f64, start: f32, count: u32) -> Vec<[f32; 3]> {
    (0..count)
        .map(|k| {
            let a = std::f64::consts::FRAC_PI_2 * f64::from(k) / f64::from(count - 1);
            let x = f64::from(start) + radius * libm::sin(a);
            [x as f32, (radius * (1.0 - libm::cos(a))) as f32, 0.0]
        })
        .collect()
}

#[test]
fn a_curve_along_a_quarter_circle_bends_a_bar_as_a_90_degree_bend_does() {
    let (n, length, h) = (64, 2.0, 0.05);
    let bar = strip(n, length, h);
    let bend = |angle| Deform::Bend {
        axis: 0,
        toward: 1,
        angle,
    };
    // Over the whole bar, and over its middle with the rest going on straight past both ends.
    for (lo, hi) in [(0.0, length), (0.5, 1.5)] {
        let boxed = |d| {
            let mut t = deform(d);
            t.scope.bounds = Some(Aabb {
                lo: Vec3::new(lo, -1.0, -1.0),
                hi: Vec3::new(hi, 1.0, 1.0),
            });
            t
        };
        let radius = f64::from(hi - lo) / std::f64::consts::FRAC_PI_2;
        let on = curve(0, Some(quarter(radius, lo, 4)), None);
        // At weight one half every turn is halved: a 45 degree bend.
        for (mesh, angle) in [(bar.clone(), FRAC_PI_2), (masked(&bar, |_| 0.5), FRAC_PI_4)] {
            let got = after(&mesh, &boxed(on.clone()));
            let want: Vec<[f64; 3]> = (after(&bar, &boxed(bend(angle))).into_iter())
                .map(wide)
                .collect();
            let off = worst(&got, &want) / f64::from(length);
            assert!(off < 1e-6, "over {lo} to {hi} at {angle}: off by {off}");
        }
    }
}

#[test]
fn a_straight_curves_twist_and_scale_turn_and_scale_the_part_as_twist_and_taper_do() {
    let bar = strip(16, 2.0, 0.3);
    let scaled = Deform::Curve(Curve {
        axis: 0,
        points: None,
        twist: None,
        scale: Some(vec![1.0, 0.5]),
    });
    let pairs = [
        (
            curve(0, None, Some(vec![0.0, FRAC_PI_2])),
            Deform::Twist {
                axis: 0,
                angle: FRAC_PI_2,
            },
        ),
        (
            scaled,
            Deform::Taper {
                axis: 0,
                amount: 0.5,
                exponent: 1.0,
            },
        ),
    ];
    for (c, d) in pairs {
        let got = after(&bar, &deform(c));
        let want: Vec<[f64; 3]> = after(&bar, &deform(d.clone()))
            .into_iter()
            .map(wide)
            .collect();
        assert!(worst(&got, &want) < 1e-6, "{d:?}: {}", worst(&got, &want));
    }
}

/// How far `p` is from the polyline through `line`.
fn off_line(p: [f64; 3], line: &[[f32; 3]]) -> f64 {
    (line.windows(2))
        .map(|w| {
            let (a, b) = (wide(w[0]), wide(w[1]));
            let (ab, ap) = (sub(b, a), sub(p, a));
            let t = (dot(ap, ab) / dot(ab, ab)).clamp(0.0, 1.0);
            gap(w[0], [0, 1, 2].map(|k| p[k] - t * ab[k]))
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_bar_on_an_s_of_four_points_follows_it_keeping_its_length_and_width() {
    let (n, length, h) = (512, 3.0, 0.05);
    let bar = strip(n, length, h);
    let s = vec![
        [0.0, 0.0, 0.0],
        [0.7, 0.35, 0.0],
        [1.4, -0.35, 0.0],
        [2.1, 0.0, 0.0],
    ];
    let got = after(&bar, &deform(curve(0, Some(s.clone()), None)));
    let centre: Vec<[f32; 3]> = (0..=n).map(|i| got[(i * 3 + 1) as usize]).collect();
    for p in &s {
        let off = off_line(wide(*p), &centre);
        assert!(off < 1e-4, "the centre line passes {off} from {p:?}");
    }
    let walked: f64 = (centre.windows(2)).map(|w| gap(w[0], wide(w[1]))).sum();
    assert!(
        (walked - 3.0).abs() < 1e-3,
        "the centre line is {walked} long"
    );
    for i in 0..=n as usize {
        let width = gap(got[3 * i], wide(got[3 * i + 2]));
        assert!((width - 0.1).abs() < 1e-6, "column {i} is {width} wide");
    }
    // No kink: each chord turns little from the one before.
    for w in centre.windows(3) {
        let (a, b, c) = (wide(w[0]), wide(w[1]), wide(w[2]));
        let (u, v) = (
            [0, 1, 2].map(|k| b[k] - a[k]),
            [0, 1, 2].map(|k| c[k] - b[k]),
        );
        let cos = (u[0] * v[0] + u[1] * v[1] + u[2] * v[2])
            / (gap(w[0], b) * gap(w[1], c)).max(f64::MIN_POSITIVE);
        assert!(cos > libm::cos(0.1), "a kink at {:?}", w[1]);
    }
}

#[test]
fn small_moves_of_the_cage_and_the_curve_move_points_little() {
    let ball = sphere(10, 0.5, 13, false);
    let size = (ball.positions.iter())
        .map(|&p| gap(p, [0.0; 3]))
        .fold(0.0, f64::max);
    let nudge = 1e-6;
    let calls = [
        Deform::Lattice(Lattice {
            soft: true,
            size: [3; 3],
            offsets: vec![Offset {
                at: [1, 2, 1],
                by: [nudge, -nudge, nudge],
            }],
        }),
        Deform::Lattice(Lattice {
            soft: false,
            size: [4; 3],
            offsets: vec![Offset {
                at: [1, 2, 3],
                by: [-nudge, nudge, nudge],
            }],
        }),
        curve(
            0,
            Some(vec![[-1.2, 0.0, 0.0], [0.0, nudge, 0.0], [1.2, 0.0, 0.0]]),
            None,
        ),
        Deform::Curve(Curve {
            axis: 2,
            points: None,
            twist: Some(vec![0.0, nudge]),
            scale: Some(vec![1.0, 1.0 + nudge]),
        }),
    ];
    let start: Vec<[f64; 3]> = ball.positions.iter().map(|&p| wide(p)).collect();
    for d in calls {
        let mut t = deform(d);
        t.scope.bounds = Some(Aabb {
            lo: Vec3::new(-1.2, -1.2, -1.2),
            hi: Vec3::new(1.2, 1.2, 1.2),
        });
        let moved = worst(&after(&ball, &t), &start) / size;
        assert!(moved < 1e-5, "{:?} moved a point {moved} of the size", t.op);
    }
}

#[test]
fn the_cage_and_the_curve_on_a_parts_own_points_land_where_they_land_on_the_part_baked() {
    let ball = sphere(10, 0.5, 17, false);
    let place = Similarity {
        translation: Vec3::new(0.3, -0.2, 0.5),
        rotation: Quat::about(Vec3::new(1.0, 2.0, 3.0).normalized(), 0.7),
        scale: 1.7,
    };
    let own = gs_protocol::PlainMesh {
        positions: (ball.positions.iter())
            .map(|&p| place.part_point(Vec3::from_array(p)).to_array())
            .collect(),
        ..ball.clone()
    };
    let axes = Quat::about(Vec3::new(-1.0, 0.5, 0.2).normalized(), 0.3);
    let size = (ball.positions.iter())
        .map(|&p| gap(p, [0.0; 3]))
        .fold(0.0, f64::max);
    let offsets = vec![
        Offset {
            at: [1, 2, 0],
            by: [0.2, 0.3, -0.1],
        },
        Offset {
            at: [0, 1, 3],
            by: [-0.1, 0.0, 0.25],
        },
    ];
    let calls = [
        Deform::Lattice(Lattice {
            soft: true,
            size: [3, 3, 4],
            offsets: offsets.clone(),
        }),
        Deform::Lattice(Lattice {
            soft: false,
            size: [3, 3, 4],
            offsets,
        }),
        Deform::Curve(Curve {
            axis: 1,
            points: Some(vec![[0.0, -1.0, 0.0], [0.3, 0.0, 0.1], [0.0, 1.0, 0.0]]),
            twist: Some(vec![0.0, 0.5, 1.0]),
            scale: Some(vec![1.0, 0.8, 1.2]),
        }),
    ];
    for d in calls {
        let t: Transform = about([0.1, 0.05, -0.2], axes, Op::Deform(d));
        let baked = after(&ball, &t);
        let placed: Vec<[f64; 3]> = (after(&own, &place.part_transform(&t)).into_iter())
            .map(|q| wide(place.point(Vec3::from_array(q)).to_array()))
            .collect();
        let off = worst(&baked, &placed) / size;
        assert!(off < 2e-6, "{:?} off by {off} of the size", t.op);
    }
}
