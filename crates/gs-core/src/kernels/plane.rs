//! Plane: pull toward a plane fitted under the brush (Clay Build, Flat Polish, Smooth Polish,
//! Trim; Clip Curve's plane comes from its gesture instead).

use super::Frame;
use crate::brush::{PlaneSpec, Side};
use crate::math::Vec3;

/// The translation of a vertex at `p` with influence `w`.
pub(super) fn plane(k: &PlaneSpec, f: &Frame, p: Vec3, w: f32, invert: bool) -> Vec3 {
    let m = if invert { -f.normal } else { f.normal };
    let d = (p - (f.origin + m * (f.imbed * f.radius))).dot(m);
    // The caps include their bound: a vertex exactly at the cap still moves.
    let capped =
        k.height.is_some_and(|h| d > h * f.radius) || k.depth.is_some_and(|h| -d > h * f.radius);
    if capped {
        return Vec3::ZERO;
    }
    // The side filter keeps only the part of the distance on the moving side.
    let d = match k.side {
        Side::Both => d,
        Side::Above => d.max(0.0),
        Side::Below => d.min(0.0),
    };
    m * (-d * (1.0 - libm::expf(-k.gain * w * f.amount)))
}
