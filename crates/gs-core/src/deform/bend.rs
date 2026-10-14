//! Bend and fold: the two operations that turn part of the shape about a line across `axis`
//! and `toward`. Each change is written in half-angle form (`1 - cos t` as `2 sin²(t/2)`, the
//! arc's chord through `sinc`), so it is exactly zero at zero and stays precise at the smallest
//! turns, where the textbook form loses every digit to the difference of near-equal numbers.

use std::cmp::Ordering;

use super::{Ready, sinc};

/// The bend of frame coordinates `c` by `turn` radians over the box's span along `axis`,
/// turning toward `toward`. The arc is fixed at the origin clamped into the box; `x` is the
/// place along it, `e` how far past the box the point lies. A point at `x` on the axis goes to
/// the arc's point `(x sinc t, x sin(t/2) sinc(t/2))` with `t = x turn / span`; one off it by
/// `y` along `toward` goes `y` along the arc's normal, and one past the box goes on along the
/// arc's end.
pub(super) fn bend(r: &Ready, c: [f64; 3], [axis, toward]: [usize; 2], turn: f64) -> [f64; 3] {
    let (a, b) = (r.lo[axis], r.hi[axis]);
    let mut d = [0.0; 3];
    if turn == 0.0 || b.partial_cmp(&a).is_none_or(Ordering::is_le) {
        return d;
    }
    let s = c[axis];
    let inside = s.clamp(a, b);
    let (x, e, y) = (inside - 0.0_f64.clamp(a, b), s - inside, c[toward]);
    let t = turn * x / (b - a);
    let (sin, half) = (libm::sin(t), libm::sin(t / 2.0));
    d[axis] = x * (sinc(t) - 1.0) - y * sin - 2.0 * e * half * half;
    d[toward] = x * half * sinc(t / 2.0) - 2.0 * y * half * half + e * sin;
    d
}

/// The fold of frame coordinates `c` by `turn`: each side of the plane across `axis` turned
/// by half of it about the line through the origin along the third axis, both toward
/// `toward`. A point within rounding of the plane stays, so the fold does not tear a point on
/// it one way in the scene and the other in the part.
pub(super) fn fold(r: &Ready, c: [f64; 3], [axis, toward]: [usize; 2], turn: f64) -> [f64; 3] {
    let mut d = [0.0; 3];
    let s = c[axis];
    if turn == 0.0 || s.abs() <= r.near(c) {
        return d;
    }
    let phi = if s > 0.0 { turn } else { -turn } / 2.0;
    let (sin, half) = (libm::sin(phi), libm::sin(phi / 2.0));
    let y = c[toward];
    d[axis] = -2.0 * s * half * half - y * sin;
    d[toward] = s * sin - 2.0 * y * half * half;
    d
}
