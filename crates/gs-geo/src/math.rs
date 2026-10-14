//! Small vector helpers for gs-geo, and Morton codes. Only exactly rounded operations
//! (`+ - * /` and `sqrt`), so results are the same on every machine; anything transcendental
//! goes through `libm`. The closest point of a triangle and `sub`, `dot` and `cross` are
//! gs-core's shared math; the other helpers here could follow them there.

use gs_core::Vec3;
use gs_core::math::closest_weights;
pub(crate) use gs_core::math::{cross, dot, sub};

/// A point or vector in `f32`.
pub(crate) type V3 = [f32; 3];

/// Squared distance between two points.
pub(crate) fn dist2(a: V3, b: V3) -> f32 {
    let d = sub(a, b);
    dot(d, d)
}

/// The point with barycentric weights `w` in triangle `t`.
pub(crate) fn at(t: &[V3; 3], w: [f32; 3]) -> V3 {
    std::array::from_fn(|i| w[0] * t[0][i] + w[1] * t[1][i] + w[2] * t[2][i])
}

/// Barycentric weights of the point of triangle `t` closest to `p`: gs-core's one search
/// (`math::closest_weights`, Ericson 5.1.5), which answers degenerate triangles too. Inline,
/// so the distance field's loop over a triangle's samples keeps it in the loop.
#[inline]
pub(crate) fn closest(p: V3, t: &[V3; 3]) -> [f32; 3] {
    closest_weights(Vec3::from_array(p), &t.map(Vec3::from_array))
}

/// Interleaves the low `bits` bits of `c`: bit `b` of axis `a` lands on bit `3b + a`.
/// Sorting by this code walks space in Z order, so neighbours stay close in memory.
pub(crate) fn morton(c: [u32; 3], bits: u32) -> u64 {
    let mut code = 0u64;
    for b in 0..bits {
        for (axis, &v) in (0u32..).zip(&c) {
            code += u64::from((v >> b) & 1) << (3 * b + axis);
        }
    }
    code
}

/// The inverse of [`morton`].
pub(crate) fn unmorton(code: u64, bits: u32) -> [u32; 3] {
    let mut c = [0u32; 3];
    for b in 0..bits {
        for (axis, v) in (0u32..).zip(&mut c) {
            *v += (((code >> (3 * b + axis)) & 1) as u32) << b;
        }
    }
    c
}
