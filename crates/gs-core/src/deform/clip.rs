//! Clip and spherize: the two operations whose weight scales the change itself, since neither
//! has an amount a fraction of which is a smaller clip or a rounder ball on its own terms.

use std::cmp::Ordering;

use super::Ready;

/// The clip of frame coordinates `c` along `axis` at `depth` inside the box's top face (the
/// bottom face when negative), the change scaled by weight `w`. The plane is clamped into the
/// box; a point past the face moves as the face does, so the part beyond keeps its shape.
pub(super) fn clip(r: &Ready, c: [f64; 3], axis: usize, depth: f32, w: f64) -> [f64; 3] {
    let mut d = [0.0; 3];
    let (a, b) = (r.lo[axis], r.hi[axis]);
    if depth == 0.0 || b.partial_cmp(&a).is_none_or(Ordering::is_lt) {
        return d;
    }
    let (s, depth) = (c[axis], f64::from(depth));
    let change = if depth > 0.0 {
        let plane = (b - depth).clamp(a, b);
        -(s.min(b) - plane).max(0.0)
    } else {
        let plane = (a - depth).clamp(a, b);
        (plane - s.max(a)).max(0.0)
    };
    d[axis] = change * w;
    d
}

/// The spherize of frame coordinates `c` by `amount` (already weighed) toward the sphere of
/// `radius` about the origin, over the kept axes alone (a cylinder when one is left out). A
/// point within rounding of the origin has no direction and stays.
pub(super) fn spherize(r: &Ready, c: [f64; 3], amount: f64, radius: f32) -> [f64; 3] {
    let kept = r.held(c);
    let length = (kept[0] * kept[0] + kept[1] * kept[1] + kept[2] * kept[2]).sqrt();
    let radius = f64::from(radius);
    if amount == 0.0 || length <= 1e-6 * (radius + r.far) {
        return [0.0; 3];
    }
    let f = amount * (radius / length - 1.0);
    kept.map(|k| f * k)
}
