//! gs-render's material model (research 11, phases 2 and 3) against physics and against the
//! slow f64 references in `gs_testkit::reference::shading`. One binary, so it links once.

mod bake;
mod integrals;
mod lit;
mod model;
mod transforms;

use gs_testkit::reference::shading::{V, unit};

/// A BRDF as gs-render's `Material` exposes it: material, normal, view, light.
pub type Brdf = fn(&gs_render::shading::Material, [f32; 3], [f32; 3], [f32; 3]) -> [f32; 3];

/// f32 to f64 and back, for handing vectors between gs-render and the references.
pub fn wide(v: [f32; 3]) -> V {
    v.map(f64::from)
}

pub fn narrow(v: V) -> [f32; 3] {
    v.map(|c| c as f32)
}

/// A view direction at cosine `mu` from the normal +z.
pub fn view(mu: f64) -> V {
    unit([(1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu])
}
