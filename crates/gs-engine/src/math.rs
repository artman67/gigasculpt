//! Small vector helpers for the engine: `sub`, `dot` and `cross` are gs-core's shared math.
//! Only exactly rounded operations (`+ - * /` and `sqrt`), so results are the same on every
//! machine.

pub(crate) use gs_core::math::{cross, dot, sub};

/// Length of a vector.
pub(crate) fn norm([x, y, z]: [f32; 3]) -> f32 {
    (x * x + y * y + z * z).sqrt()
}
