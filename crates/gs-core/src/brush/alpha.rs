//! How a brush uses an alpha: the `[tip.alpha]` table (decision 0018). The picture itself never
//! changes; everything here is the brush's. `crate::alpha` turns picture plus these settings
//! into the pyramid a stroke samples.

use serde::{Deserialize, Serialize};

use crate::curve::Curve;

/// The picture a brush stamps with, and how it uses it. The tone settings (`flip` to
/// `radial_fade`) run in this fixed order: flips and quarter turns, invert, maximize, blur,
/// noise, contrast, curve, radial fade. `tiles` and `mid` apply when the stroke reads it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AlphaSpec {
    /// The picture: a library name (`square`) or `blake3:` and the 64 hex digits of its hash.
    pub image: String,
    /// The value that counts as zero, 0 to 1. At 0 the alpha only raises; on the Offset kernel
    /// values below it carve (decision 0018, choices 1 and 2).
    #[serde(default)]
    pub mid: f32,
    /// Mirror the picture.
    #[serde(default)]
    pub flip: Flip,
    /// Quarter turns clockwise, 0 to 3. A picture that is not square stretches to the square.
    #[serde(default)]
    pub turn: u8,
    /// Swap dark and light.
    #[serde(default)]
    pub invert: bool,
    /// Stretch the darkest value to 0 and the lightest to 1.
    #[serde(default)]
    pub maximize: bool,
    /// Gaussian blur: its sigma in pixels of a 256-pixel-wide copy, 0 to 64.
    #[serde(default)]
    pub blur: f32,
    /// Tileable noise added, 0 to 1.
    #[serde(default)]
    pub noise: f32,
    /// Noise cells across the picture, 1 to 1024.
    #[serde(default = "sixteen")]
    pub noise_scale: u32,
    /// Which noise.
    #[serde(default)]
    pub noise_seed: u32,
    /// Steepen around the middle value, 0 to 16; 1 changes nothing.
    #[serde(default = "one")]
    pub contrast: f32,
    /// Remap values, dark (x = 0) to light.
    #[serde(default = "Curve::linear")]
    pub curve: Curve,
    /// Fade toward the rim of the circle inside the picture: values times `(1 - d)` to this
    /// power, `d` the distance from the center in half-widths; 0 (off) to 8.
    #[serde(default)]
    pub radial_fade: f32,
    /// Repeat the picture this many times across and down the footprint, 1 to 64 each: one
    /// setting for an even tiling and for separate counts across and down.
    #[serde(default = "single")]
    pub tiles: [u32; 2],
    /// Adaptive Map Size, 0 to 1: a dab reads the finest pyramid level whose texel spacing is
    /// at least this times half the vertex spacing, so 0 reads the full picture (decision
    /// 0058). Left out of the text at 1, so a preset without it keeps its hash.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub adaptive: f32,
}

/// Mirroring a picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Flip {
    #[default]
    None,
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
    Both,
}

fn sixteen() -> u32 {
    16
}
fn one() -> f32 {
    1.0
}
#[expect(
    clippy::trivially_copy_pass_by_ref,
    clippy::float_cmp,
    reason = "serde's skip_serializing_if passes a reference; the default is the exact 1"
)]
fn is_one(x: &f32) -> bool {
    *x == 1.0
}
fn single() -> [u32; 2] {
    [1, 1]
}

/// What a picture name may start with to name it by hash.
pub(crate) const HASH_PREFIX: &str = "blake3:";

impl AlphaSpec {
    /// The settings for `image` with every tone setting neutral.
    pub fn plain(image: &str) -> AlphaSpec {
        AlphaSpec {
            image: image.into(),
            mid: 0.0,
            flip: Flip::None,
            turn: 0,
            invert: false,
            maximize: false,
            blur: 0.0,
            noise: 0.0,
            noise_scale: sixteen(),
            noise_seed: 0,
            contrast: 1.0,
            curve: Curve::linear(),
            radial_fade: 0.0,
            tiles: single(),
            adaptive: one(),
        }
    }

    /// The hash of the tone settings alone (not the picture, `mid`, `tiles` or `adaptive`,
    /// which the stroke applies as it reads): with the picture's hash, the key of a processed
    /// pyramid.
    pub fn tone_key(&self) -> String {
        let tone = AlphaSpec {
            image: String::new(),
            mid: 0.0,
            tiles: single(),
            adaptive: one(),
            ..self.clone()
        };
        let text = toml::to_string(&tone).unwrap_or_default();
        blake3::hash(text.as_bytes()).to_hex().to_string()
    }

    /// The picture's hash when `image` names it by hash.
    pub fn hash_named(&self) -> Option<&str> {
        self.image.strip_prefix(HASH_PREFIX)
    }
}
