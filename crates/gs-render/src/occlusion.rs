//! Ambient occlusion's settings (catalog item `render.live-ambient-occlusion`, decision 0019):
//! GTAO in the form of Intel's `XeGTAO`, from the geometry pass's depth and normals, with a reach
//! in scene units so a crease shades the same at any zoom. The passes are in
//! `shaders/occlusion.wgsl`; the lighting pass applies the result to environment light only.
//! The session sets the strength and radius with `view.display` (decision 0019, settings and
//! controls); every look but normals takes them. A standard material's cavity settings scale
//! them in MatCap and Lit shading (decision 0043, step (c)).

use gs_core::material::Cavity;

use crate::math::{dot, scale, sub};
use crate::mesh::Bounds;

/// The reach of the default radius: this fraction of the radius of the sphere around
/// everything drawn. A free choice for the maintainer to judge; Blender's World cavity default
/// (0.2 units on its 2-unit cube) is about the same fraction.
pub const DEFAULT_RADIUS_FRACTION: f32 = 0.1;

/// How a view darkens what its surroundings' light cannot reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occlusion {
    /// How far, in scene units, a surface looks for what blocks its light. `None`: a tenth of
    /// the radius of the sphere around everything drawn, so it scales with the model.
    pub radius: Option<f32>,
    /// 0 (no occlusion, and the passes are skipped) to 1 (full).
    pub strength: f32,
    /// The drawn material's cavity settings: Intensity scales the strength, Radius the radius,
    /// and Colorize gives back the light occlusion takes. Neutral in Clay, Zebra, Normals and
    /// Flat shading.
    pub cavity: Cavity,
}

impl Default for Occlusion {
    /// The default radius at full strength.
    fn default() -> Self {
        Occlusion {
            radius: None,
            strength: 1.0,
            cavity: Cavity::NEUTRAL,
        }
    }
}

impl Occlusion {
    /// No occlusion.
    pub const OFF: Occlusion = Occlusion {
        radius: None,
        strength: 0.0,
        cavity: Cavity::NEUTRAL,
    };

    /// The radius in scene units for a scene whose chunk bounds have these corners: the set
    /// radius, or the default fraction of the bounding sphere. 0 turns occlusion off.
    pub fn radius_for(&self, points: &[[f32; 3]]) -> f32 {
        if self.strength.is_nan() || self.strength <= 0.0 {
            return 0.0;
        }
        if let Some(r) = self.radius {
            return r.max(0.0);
        }
        let mut b = Bounds::EMPTY;
        for &p in points {
            b.include(p);
        }
        if points.is_empty() {
            return 0.0;
        }
        let half = scale(sub(b.max, b.min), 0.5);
        DEFAULT_RADIUS_FRACTION * dot(half, half).sqrt()
    }
}

/// Slices and steps per side of the occlusion pass, and frame pixels per occlusion texel along
/// each side, by tier (ARCHITECTURE 11; decision 0019): `XeGTAO`'s High at full resolution on
/// the discrete GPU and on lavapipe (so goldens show the full quality); on the integrated GPU
/// High's three slices with two steps each at half resolution, which the lighting pass scales
/// up guided by depth. Two slices (Medium) left a single sample too noisy at silhouettes.
pub(crate) fn quality(integrated: bool) -> (u32, u32, u32) {
    if integrated { (3, 2, 2) } else { (3, 3, 1) }
}
