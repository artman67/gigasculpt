//! Small vector helpers for normals, culling bounds and the MatCap baker: gs-render's home for
//! vector math until gs-core has the shared math module (gs-engine keeps its own helpers for
//! the same reason; gs-render cannot depend on the engine). The camera and its matrices live in
//! gs-core (`gs_core::camera`). The vector helpers work in f32 and, where culling needs the
//! headroom, f64.
//!
//! Only `+ - * / sqrt` here, which are exact IEEE operations, so results do not depend on the
//! CPU. Trigonometry goes through `libm` at the call sites.

use std::ops::{Add, Mul, Sub};

pub(crate) type V3 = [f32; 3];
pub(crate) use gs_core::camera::M4;

/// f32 or f64.
pub(crate) trait Real:
    Copy + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> + std::iter::Sum
{
}

impl Real for f32 {}
impl Real for f64 {}

pub(crate) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub<T: Real>(a: [T; 3], b: [T; 3]) -> [T; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}

pub(crate) fn scale<T: Real>(a: [T; 3], s: T) -> [T; 3] {
    a.map(|x| x * s)
}

pub(crate) fn dot<T: Real>(a: [T; 3], b: [T; 3]) -> T {
    a.iter().zip(b).map(|(&x, y)| x * y).sum()
}

pub(crate) fn cross<T: Real>(a: [T; 3], b: [T; 3]) -> [T; 3] {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}

/// `x > limit`, and false when either is NaN: the form every threshold test here takes, so a
/// NaN never passes one.
pub(crate) fn above<T: PartialOrd + Copy>(x: T, limit: T) -> bool {
    matches!(x.partial_cmp(&limit), Some(std::cmp::Ordering::Greater))
}

/// Unit vector along `a`, or `fallback` when `a` has no length.
pub(crate) fn normalize_or(a: V3, fallback: V3) -> V3 {
    let len = dot(a, a).sqrt();
    if len > 0.0 {
        scale(a, 1.0 / len)
    } else {
        fallback
    }
}

/// `a * b` for column-major matrices (`m[column][row]`), in f64.
pub(crate) fn mul4(a: &M4, b: &M4) -> M4 {
    std::array::from_fn(|c| {
        std::array::from_fn(|r| {
            let sum: f64 = (0..4)
                .map(|k| f64::from(a[k][r]) * f64::from(b[c][k]))
                .sum();
            sum as f32
        })
    })
}
