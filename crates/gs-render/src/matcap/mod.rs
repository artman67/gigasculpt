//! MatCaps (research 11, section 5): a MatCap is a cached bake of the shared material model
//! (`crate::shading`) on a ball under an environment, stored as two linear HDR layers, D and
//! S (see [`bake`]). Looks are recipe files (`assets/matcaps/<name>.toml`, catalog item
//! `material.look-recipe`); [`Library`] reads them and [`SHIPPED`] holds every shipped one,
//! all of them gs-core's (`gs_core::material`, decision 0043) and named here as before.
//! An 8-bit MatCap image a user brings ([`MatCapImage`]) is a separate, simple path: an image
//! looked up by view normal and shown as the file looks.
//!
//! Sphere convention: orthographic view down -Z, unit sphere inscribed in the square image.
//! Pixel (u, v), with u and v in -1..1 and v growing downward, holds the shading for the normal
//! `(u, -v, sqrt(1 - u^2 - v^2))`. Outside the circle the silhouette's shading continues
//! outward, so filtering at the edge never pulls in anything else.

mod bake;

pub use bake::{Options, bake, bake_under};
pub use gs_core::material::{Bake, DEFAULT_LOOK, Library, Look, Recipe, Refused, SHIPPED};

use crate::error::RenderError;

/// A square MatCap image as a user brings it: 16-bit sRGB-encoded RGB and 16-bit
/// linear alpha, top row first. Display-referred: it shows as the file looks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatCapImage {
    pub size: u32,
    pub pixels: Vec<[u16; 4]>,
}

impl MatCapImage {
    /// A 16-bit RGBA PNG tagged sRGB.
    pub fn png(&self) -> Result<Vec<u8>, RenderError> {
        let bytes: Vec<u8> = self
            .pixels
            .iter()
            .flatten()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        crate::image::encode_png(self.size, self.size, png::BitDepth::Sixteen, &bytes)
    }
}

/// A baked look: the D and S layers as half floats (RGBA16F bits, alpha 1), top row first,
/// the albedo that multiplies D when drawn (the look's base color, linear), the look's
/// specular roughness, which sets how much ambient occlusion dims S (decision 0019), and its
/// metalness, which sets how much paint tints S (research 11, R7 in research 27).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Baked {
    pub size: u32,
    pub d: Vec<[u16; 4]>,
    pub s: Vec<[u16; 4]>,
    pub albedo: [u32; 3],
    pub roughness: u32,
    pub metalness: u32,
}

impl Baked {
    /// The look's albedo as linear floats.
    pub fn albedo(&self) -> [f32; 3] {
        self.albedo.map(f32::from_bits)
    }

    /// The look's specular roughness (perceptual, 0 to 1).
    pub fn roughness(&self) -> f32 {
        f32::from_bits(self.roughness)
    }

    /// The look's metalness (0 to 1).
    pub fn metalness(&self) -> f32 {
        f32::from_bits(self.metalness)
    }

    /// D and S at pixel (x, y), linear.
    pub fn layers(&self, x: u32, y: u32) -> ([f32; 3], [f32; 3]) {
        let at = (y * self.size + x) as usize;
        let rgb = |p: [u16; 4]| [p[0], p[1], p[2]].map(|h| half::f16::from_bits(h).to_f32());
        (rgb(self.d[at]), rgb(self.s[at]))
    }

    /// The light a white-lit pixel (x, y) shows: `albedo x D + S`.
    pub fn radiance(&self, x: u32, y: u32) -> [f32; 3] {
        let ((d, s), a) = (self.layers(x, y), self.albedo());
        std::array::from_fn(|k| a[k] * d[k] + s[k])
    }
}

impl crate::GpuMatCap {
    /// The same MatCap with Cavity Transition `t`, -1 to 1 (catalog item
    /// `material.cavity-shading`): a flat-plus-crease pair blends toward its crease image by
    /// one minus the occlusion's visibility plus `t`, so -1 shows only the flat image and +1
    /// only the crease image. 0 (the default) leaves the blend as it is; a MatCap without a
    /// crease image ignores it.
    #[must_use]
    pub fn transition(mut self, t: f32) -> Self {
        self.transition = t.clamp(-1.0, 1.0);
        self
    }
}
