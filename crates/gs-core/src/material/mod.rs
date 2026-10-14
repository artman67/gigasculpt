//! Materials as data (decision 0043): a material's physical numbers with `OpenPBR`'s names
//! (research 11, 12.1), the look recipe that adds the environment lighting it and how it bakes
//! (catalog item `material.look-recipe`), and the shipped looks and environments. The engine
//! keeps the palette of these and the renderer draws them; gs-render evaluates the BRDF they
//! define.

mod library;
mod recipe;

pub use library::{DEFAULT_LOOK, ENVIRONMENTS, Library, Look, Refused, SHIPPED};
pub use recipe::{Bake, Cavity, MaterialKind, Recipe, parse_toml};

use serde::{Deserialize, Serialize};

use crate::columns::srgb_to_linear;

/// Linear-light RGB.
pub type Rgb = [f32; 3];

/// A color as a look file writes it: `"#rrggbb"` (sRGB, decoded once on load) or
/// `[r, g, b]` (linear, as measured-material databases give them). Written linear, so it reads
/// back exactly.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "ColorText", into = "Rgb")]
pub struct Color(pub Rgb);

impl From<Color> for Rgb {
    fn from(c: Color) -> Rgb {
        c.0
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ColorText {
    Hex(String),
    Linear(Rgb),
}

impl TryFrom<ColorText> for Color {
    type Error = String;
    fn try_from(text: ColorText) -> Result<Color, String> {
        let hex = match text {
            ColorText::Linear(rgb) => return Ok(Color(rgb)),
            ColorText::Hex(hex) => hex,
        };
        let digits = hex
            .strip_prefix('#')
            .filter(|h| h.len() == 6 && h.is_ascii())
            .ok_or_else(|| format!("color {hex:?} is not #rrggbb or [r, g, b]"))?;
        let channel = |i: usize| {
            u8::from_str_radix(&digits[i..i + 2], 16)
                .map(|c| srgb_to_linear(f32::from(c) / 255.0))
                .map_err(|_| format!("color {hex:?} is not #rrggbb"))
        };
        Ok(Color([channel(0)?, channel(2)?, channel(4)?]))
    }
}

impl Color {
    pub const WHITE: Color = Color([1.0; 3]);
}

/// The physical description of a surface. Every field has `OpenPBR`'s name and default.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Material {
    /// Diffuse albedo; for a metal, its reflectance at normal incidence.
    pub base_color: Color,
    /// 0 dielectric, 1 metal; in between only for dusty or dirty metal.
    pub base_metalness: f32,
    /// Scales the specular reflectance.
    pub specular_weight: f32,
    /// Tints a dielectric's reflectance at normal incidence; for a metal, its F82 edge tint.
    pub specular_color: Color,
    /// Perceptual roughness; the GGX alpha is its square.
    pub specular_roughness: f32,
    /// A dielectric's index of refraction: 1.5 reflects 4% at normal incidence.
    pub specular_ior: f32,
    /// How much of a clear coat covers the base.
    pub coat_weight: f32,
    /// What the coat absorbs on the way to the base and back.
    pub coat_color: Color,
    pub coat_roughness: f32,
    pub coat_ior: f32,
    /// How much of the dielectric body scatters under the surface instead of reflecting at it.
    pub subsurface_weight: f32,
    /// Mean free path of light under the surface, in scene units (shipped looks: millimeters).
    pub subsurface_radius: f32,
    /// Per-channel multiplier on the radius (red travels furthest in skin and wax).
    pub subsurface_radius_scale: Rgb,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            base_color: Color([0.8; 3]),
            base_metalness: 0.0,
            specular_weight: 1.0,
            specular_color: Color::WHITE,
            specular_roughness: 0.3,
            specular_ior: 1.5,
            coat_weight: 0.0,
            coat_color: Color::WHITE,
            coat_roughness: 0.0,
            coat_ior: 1.6,
            subsurface_weight: 0.0,
            subsurface_radius: 1.0,
            subsurface_radius_scale: [1.0, 0.5, 0.25],
        }
    }
}

impl Material {
    /// The physical ranges, each error naming its key: albedos and weights 0..1, roughness
    /// 0..1, IOR 1..3, radii 0 or more.
    pub fn check(&self) -> Result<(), String> {
        let unit = [
            ("base_metalness", self.base_metalness),
            ("specular_weight", self.specular_weight),
            ("specular_roughness", self.specular_roughness),
            ("coat_weight", self.coat_weight),
            ("coat_roughness", self.coat_roughness),
            ("subsurface_weight", self.subsurface_weight),
        ];
        let colors = [
            ("base_color", self.base_color.0),
            ("specular_color", self.specular_color.0),
            ("coat_color", self.coat_color.0),
        ];
        let inside = |v: f32, lo: f32, hi: f32| (lo..=hi).contains(&v);
        for (key, v) in unit {
            if !inside(v, 0.0, 1.0) {
                return Err(format!("{key}: {v} is outside 0 to 1"));
            }
        }
        for (key, c) in colors {
            if !c.iter().all(|&v| inside(v, 0.0, 1.0)) {
                return Err(format!("{key}: {c:?} is outside 0 to 1"));
            }
        }
        for (key, v) in [
            ("specular_ior", self.specular_ior),
            ("coat_ior", self.coat_ior),
        ] {
            if !inside(v, 1.0, 3.0) {
                return Err(format!("{key}: {v} is outside 1 to 3"));
            }
        }
        let radii = self
            .subsurface_radius_scale
            .map(|s| s * self.subsurface_radius);
        if !radii.iter().all(|&r| inside(r, 0.0, f32::MAX)) {
            return Err(format!("subsurface_radius: {radii:?} must be 0 or more"));
        }
        Ok(())
    }
}
