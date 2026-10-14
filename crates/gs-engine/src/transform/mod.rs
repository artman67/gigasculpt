//! Transforms of a part (decision 0027): `transform.apply`, the one command behind the gizmo's
//! drags, agents' moves and turns, and later the Deformation sliders. gs-core's `transform`
//! does the work, as a kernel step or a new part transform (`route`, decision 0029); this module
//! turns the call's arguments into it.

mod apply;
mod ops;
mod route;

pub(crate) use apply::ExactFrame;
pub(crate) use apply::{FrameArg, NO_CHANGE, OpArg, TransformApply, apply, finite, unit_quat};
pub(crate) use ops::{BoxArg, GradientArg};
pub(crate) use route::{Reads, Route, edit_points, placed, route};
