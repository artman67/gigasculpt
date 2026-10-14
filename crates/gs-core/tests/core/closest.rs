//! The closest point of a triangle (`math::closest_on_triangle`, Ericson 5.1.5), the one search
//! the projection between surfaces, Live Detail's region and voxel remesh's distances share:
//! a differential check against brute force in `f64`. A degenerate triangle (two or three
//! corners in one place, three on one line) is a segment or a point, and must still give finite
//! weights that sum to 1 and its true closest point, the nearest point of its three edges.

use gs_core::Vec3;
use gs_core::math::closest_on_triangle;
use gs_testkit::measure::{P, add, cross, dot, norm, scale, sub};

use crate::common::{mix, unit};

/// The distance from `p` to the segment from `a` to `b` (its one point when `a` is `b`).
fn segment(p: P, a: P, b: P) -> f64 {
    let e = sub(b, a);
    let ee = dot(e, e);
    let s = if ee > 0.0 {
        (dot(sub(p, a), e) / ee).clamp(0.0, 1.0)
    } else {
        0.0
    };
    norm(sub(p, add(a, scale(e, s))))
}

/// The distance from `p` to triangle `t` by brute force: the nearest of its three edges, or its
/// plane where `p` lies over it. A triangle with no area has no plane, only its edges.
fn brute_force(p: Vec3, t: &[Vec3; 3]) -> f64 {
    let [p, a, b, c] = [p, t[0], t[1], t[2]].map(|v| v.to_array().map(f64::from));
    let sides = [(a, b), (b, c), (c, a)];
    let edges = sides.iter().map(|&(x, y)| segment(p, x, y));
    let edge = edges.fold(f64::INFINITY, f64::min);
    let n = cross(sub(b, a), sub(c, a));
    let over =
        dot(n, n) > 0.0 && (sides.iter()).all(|&(x, y)| dot(n, cross(sub(y, x), sub(p, x))) >= 0.0);
    if over {
        edge.min(dot(n, sub(p, a)).abs() / norm(n))
    } else {
        edge
    }
}

/// The contract at `p` on `t`: the point is as near as brute force finds, its weights are
/// finite, in [0, 1] and sum to 1 within 1e-6, and they rebuild the point.
fn check(p: Vec3, t: [Vec3; 3]) {
    let (q, w) = closest_on_triangle(p, &t);
    let (got, near) = (f64::from((q - p).len2()).sqrt(), brute_force(p, &t));
    assert!(
        (got - near).abs() <= 1e-5,
        "{t:?} from {p:?}: {got} away, brute force {near}"
    );
    let unit_range = -1e-6..=1.0 + 1e-6;
    assert!(
        w.iter().all(|x| unit_range.contains(x)),
        "{t:?} from {p:?}: {w:?}"
    );
    let sum: f32 = w.iter().sum();
    assert!(
        (sum - 1.0).abs() <= 1e-6,
        "{t:?} from {p:?}: {w:?} sums to {sum}"
    );
    let rebuilt = t[0] * w[0] + t[1] * w[1] + t[2] * w[2];
    assert!(
        (rebuilt - q).len2() <= 1e-10,
        "{t:?} from {p:?}: {w:?} rebuild {rebuilt:?}, not {q:?}"
    );
}

/// `n` points around `t`: within 1.5 of its corners' mean on each axis.
fn around(t: &[Vec3; 3], seed: u64, n: u64) -> impl Iterator<Item = Vec3> {
    let mid = (t[0] + t[1] + t[2]) * (1.0 / 3.0);
    (0..n).map(move |i| {
        let h = |k: u64| 1.5 * unit(mix(seed * 100_000 + 3 * i + k));
        mid + Vec3::new(h(0), h(1), h(2))
    })
}

/// Two corners in one place leave an edge of no length, so the triangle is the segment between
/// its two points. The first case is Live Detail's face from (0.5, 0.1, 0), once its own test
/// (which took anything from the segment's distance to the corner's): the segment is 0.1 away
/// (0.01 squared), the corner at the origin 0.51 (0.26 squared), and a dab's region takes the
/// face only by the true one.
#[test]
fn a_triangle_with_two_or_three_corners_in_one_place_gives_its_true_closest_point() {
    let (a, c) = (Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0));
    for t in [[a, a, c], [a, c, a], [c, a, a]] {
        check(Vec3::new(0.5, 0.1, 0.0), t);
    }
    let (x, y) = (Vec3::new(0.2, -0.3, 0.7), Vec3::new(1.1, 0.4, -0.6));
    let cases = [[x, x, y], [x, y, x], [y, x, x], [x, x, x]];
    for (seed, t) in (0..).zip(cases) {
        for p in around(&t, seed, 300) {
            check(p, t);
        }
    }
}

/// Three corners on one line: the triangle is a segment and has no inside. Along an axis the
/// arithmetic is exact; on a tilted line rounding leaves the inside's shares a few units in the
/// last place each, which made up a point inside from noise, or divided by their sum.
#[test]
fn a_triangle_with_its_corners_on_one_line_gives_its_true_closest_point() {
    let lines = [
        (Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)),
        (Vec3::new(0.1, 0.2, -0.4), Vec3::new(0.3, -0.7, 1.1)),
        (Vec3::new(-0.5, 0.3, 0.2), Vec3::new(0.6, 0.5, 0.4)),
    ];
    // Every order of the corners, so the middle one takes each slot.
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut seed = 10;
    for (o, d) in lines {
        let on = [0.0, 0.37, 1.9].map(|s| o + d * s);
        for [i, j, k] in orders {
            let t = [on[i], on[j], on[k]];
            seed += 1;
            for p in around(&t, seed, 300) {
                check(p, t);
            }
        }
    }
}

/// Any well-shaped triangle (at least a twentieth as high as it is long): random corners and
/// points, against brute force.
#[test]
fn the_closest_point_of_a_random_triangle_is_the_one_brute_force_finds() {
    let mut checked = 0;
    for i in 0..600 {
        let c = |k: u64| {
            Vec3::new(
                unit(mix(9 * i + k)),
                unit(mix(9 * i + k + 3)),
                unit(mix(9 * i + k + 6)),
            )
        };
        let t = [c(0), c(1), c(2)];
        let [a, b, c] = t.map(|v| v.to_array().map(f64::from));
        let longest = [sub(b, a), sub(c, b), sub(a, c)].map(|e| dot(e, e));
        let longest = longest.into_iter().fold(0.0, f64::max);
        if norm(cross(sub(b, a), sub(c, a))) < longest / 20.0 {
            continue;
        }
        for p in around(&t, 1_000 + i, 8) {
            check(p, t);
            checked += 1;
        }
    }
    assert!(checked > 4_000, "only {checked} points checked");
}
