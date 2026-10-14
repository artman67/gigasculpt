//! Offset: push along up to two directions (Draw, Clay, Sharp Crease, Pinch, Inflate).

use super::Frame;
use crate::brush::{Direction, OffsetSpec, OffsetTerm};
use crate::math::Vec3;

/// The translation of a vertex at `p` with normal `n` and influence `w`: the preset's terms
/// and the dab's Brush Modifier term. A negative `w` (an alpha below its mid value) pushes the
/// other way.
pub(super) fn offset(k: &OffsetSpec, f: &Frame, p: Vec3, n: Vec3, w: f32, invert: bool) -> Vec3 {
    let mut d = Vec3::ZERO;
    for t in &k.terms {
        d += term(t, f, p, n, w, invert);
    }
    if let Some(t) = &f.modifier {
        d += modifier(t, f, p, n, w, invert);
    }
    d
}

/// The Brush Modifier's term, out of line so the loop above compiles as it does without one.
#[inline(never)]
fn modifier(t: &OffsetTerm, f: &Frame, p: Vec3, n: Vec3, w: f32, invert: bool) -> Vec3 {
    term(t, f, p, n, w, invert)
}

/// One term's translation.
fn term(t: &OffsetTerm, f: &Frame, p: Vec3, n: Vec3, w: f32, invert: bool) -> Vec3 {
    let gain = if invert { -t.gain } else { t.gain };
    // An exponent of 1 (the default, and most presets') is `w` itself, bit for bit: libm's powf
    // returns `x` for it, so skipping the call changes nothing but the time.
    let power = if t.exponent.to_bits() == 1f32.to_bits() {
        w
    } else if w >= 0.0 {
        libm::powf(w, t.exponent)
    } else {
        -libm::powf(-w, t.exponent)
    };
    let a = gain * power * f.amount;
    let axis = |x, y, z| f.xf.apply(Vec3::new(x, y, z));
    match t.dir {
        Direction::ToCenter => {
            let mut v = f.center - p;
            if t.tangent {
                v = v - f.normal * f.normal.dot(v);
            }
            v * (1.0 - libm::expf(-a))
        }
        Direction::AreaNormal => f.normal * (a * f.radius),
        Direction::VertexNormal => n * (a * f.radius),
        Direction::View => f.view * (-a * f.radius),
        Direction::X => axis(1.0, 0.0, 0.0) * (a * f.radius),
        Direction::Y => axis(0.0, 1.0, 0.0) * (a * f.radius),
        Direction::Z => axis(0.0, 0.0, 1.0) * (a * f.radius),
    }
}
