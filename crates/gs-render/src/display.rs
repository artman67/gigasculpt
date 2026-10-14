//! How light becomes screen pixels (research 11, section 6; decision 0019): the view's
//! exposure, view transform and dither, and the 8-bit encoding. The display pass
//! (`shaders/display.wgsl`) applies them per pixel by its kind: physical light gets the
//! exposure and the view transform; display-referred MatCaps (8-bit images as they look, such
//! as imported ones) get the exposure only, so at exposure 0 they show exactly as the file
//! does; display aids (clay, zebra, normals) are already display colors and are shown as they
//! are, with no dither. The view transforms are averaged after they are applied, so bright
//! edges resolve as display light.
//!
//! Research 11 sent display-referred images through the inverse of PBR Neutral instead, but
//! that transform cannot produce bright saturated colors (its shoulder lifts every channel
//! toward the peak), so an 8-bit pixel with one channel at 255, such as a pure red, would come
//! back almost white.

use crate::color::{agx, linear_to_srgb, pbr_neutral, srgb_to_linear};

/// How scene-linear light becomes display light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewTransform {
    /// Khronos PBR Neutral, the default: base colors show as set under unit light and
    /// highlights compress toward white, desaturating as they go.
    #[default]
    PbrNeutral,
    /// `AgX` (Blender's default since 4.0), after Filament's operator: brights go toward white
    /// as on film. No exact inverse.
    Agx,
    /// Clamps to 0..1 and nothing else, for colors that are already display-referred.
    Standard,
}

impl ViewTransform {
    /// Every transform, in menu order.
    pub const ALL: [ViewTransform; 3] = [Self::PbrNeutral, Self::Agx, Self::Standard];

    /// `pbr-neutral`, `agx` or `standard`.
    pub fn name(self) -> &'static str {
        match self {
            Self::PbrNeutral => "pbr-neutral",
            Self::Agx => "agx",
            Self::Standard => "standard",
        }
    }

    /// The transform called `name`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }

    /// Scene-linear Rec. 709 to display-linear 0..1, as the display pass computes it.
    pub fn apply(self, rgb: [f32; 3]) -> [f32; 3] {
        match self {
            Self::PbrNeutral => pbr_neutral(rgb),
            Self::Agx => agx(rgb),
            Self::Standard => rgb.map(|c| c.clamp(0.0, 1.0)),
        }
    }
}

/// How a frame's light is shown: exposure, view transform and dither.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    /// In stops: +1 doubles the light before the view transform.
    pub exposure: f32,
    pub transform: ViewTransform,
    /// Triangular noise of about one 8-bit level on covered pixels, against banding in dark
    /// gradients. The pattern is fixed per pixel, so frames stay byte-identical run to run.
    pub dither: bool,
}

impl Default for Display {
    /// Exposure 0, PBR Neutral, dithered.
    fn default() -> Self {
        Display {
            exposure: 0.0,
            transform: ViewTransform::PbrNeutral,
            dither: true,
        }
    }
}

impl Display {
    /// Colors passed through unchanged: what display aids are shown with.
    pub const RAW: Display = Display {
        exposure: 0.0,
        transform: ViewTransform::Standard,
        dither: false,
    };

    /// The light multiplier the exposure stands for.
    pub fn gain(self) -> f32 {
        libm::exp2f(self.exposure)
    }

    /// Display-linear 0..1 for scene-linear `rgb`, as the display pass computes it (before
    /// dither and 8-bit encoding).
    pub fn show(self, rgb: [f32; 3]) -> [f32; 3] {
        let gain = self.gain();
        self.transform.apply(rgb.map(|c| c * gain))
    }
}

/// The 8-bit sRGB value `display` (0..1, display-linear) encodes to, as the display pass
/// writes it without dither.
pub fn encode(display: [f32; 3]) -> [u8; 3] {
    display.map(|c| (linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8)
}

/// Display-linear 0..1 for an 8-bit sRGB value.
pub fn decode(srgb: [u8; 3]) -> [f32; 3] {
    srgb.map(|c| srgb_to_linear(f32::from(c) / 255.0))
}
