//! Twist: each slice across an axis turned about it in proportion to its place along the box.

use super::Ready;

/// The twist of frame coordinates `c` about `axis` by `turn` radians over the box's span: a
/// slice at place `u` turns by `turn u`, in half-angle form so a zero turn changes nothing.
pub(super) fn twist(r: &Ready, c: [f64; 3], axis: usize, turn: f64) -> [f64; 3] {
    let mut d = [0.0; 3];
    let theta = turn * r.place(axis, c[axis]);
    if theta == 0.0 {
        return d;
    }
    let (p, q) = ((axis + 1) % 3, (axis + 2) % 3);
    let (sin, half) = (libm::sin(theta), libm::sin(theta / 2.0));
    d[p] = -2.0 * c[p] * half * half - c[q] * sin;
    d[q] = c[p] * sin - 2.0 * c[q] * half * half;
    d
}
