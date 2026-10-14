//! Taper and shear: each slice across an axis scaled, or moved sideways, by its place along
//! the box.

use super::Ready;

/// The taper of frame coordinates `c` along `axis` by `amount` (already weighed): the slice at
/// place `u` scaled about the axis by `1 - amount sgn(u) |u|^exponent`.
pub(super) fn taper(r: &Ready, c: [f64; 3], axis: usize, amount: f64, exponent: f32) -> [f64; 3] {
    let mut d = [0.0; 3];
    let u = r.place(axis, c[axis]);
    let g = amount * libm::pow(u.abs(), f64::from(exponent)).copysign(u);
    if g == 0.0 {
        return d;
    }
    for (k, d) in d.iter_mut().enumerate() {
        if k != axis {
            *d = -g * c[k];
        }
    }
    d
}

/// The shear of frame coordinates `c` along `toward` by `amount` (already weighed) times the
/// place `u` along `axis`, or its smoothstep `u² (3 - 2u)`, the sign kept.
pub(super) fn shear(
    r: &Ready,
    c: [f64; 3],
    [axis, toward]: [usize; 2],
    amount: f64,
    smooth: bool,
) -> [f64; 3] {
    let mut d = [0.0; 3];
    let u = r.place(axis, c[axis]);
    let g = if smooth {
        let a = u.abs();
        (a * a * (3.0 - 2.0 * a)).copysign(u)
    } else {
        u
    };
    d[toward] = amount * g;
    d
}
