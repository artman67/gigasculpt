//! What every dab reads beside its kernel: the tip (falloff, hardness, Falloff and alpha), how
//! high the brush sits, how the dab frame reads the surface, the pen pressure curves, the post
//! effects and the automasks. What each means is specified in `crate::brush`.

use serde::{Deserialize, Serialize};

use super::{AlphaSpec, SmoothMode};
use crate::curve::Curve;
use crate::falloff::{Profile, Tip};

/// How high the brush sits against the surface, the Depth Mask's band along the normal and
/// Infinite Depth's reach (decision 0053). Each setting but `imbed` is left out of the preset's
/// text while it is its default, so a preset without them keeps its hash.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Depth {
    /// The Plane kernel's plane above the area center, -1 to 1 radii.
    pub imbed: f32,
    /// The Depth Mask: weights go by height along the area normal, inside a band from Inner to
    /// Outer Depth.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mask: bool,
    /// Inner Depth: how far the band reaches below the surface point, -1 to 0 radii.
    #[serde(skip_serializing_if = "is_minus_one")]
    pub inner: f32,
    /// Outer Depth: how far the band reaches above it, 0 to 1 radii.
    #[serde(skip_serializing_if = "is_one")]
    pub outer: f32,
    /// Infinite Depth: the reach runs without end along the view, or along the model axes
    /// switched on in `axes`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub infinite: bool,
    /// Infinite Depth's X, Y and Z switches.
    #[serde(skip_serializing_if = "no_axis")]
    pub axes: [bool; 3],
    /// The Depth Curve: the band's weight from the surface point (`x = 0`) to either edge
    /// (`x = 1`), values 0 to 1. Last, since a curve can be written as a table.
    #[serde(skip_serializing_if = "is_flat")]
    pub curve: Curve,
}

impl Default for Depth {
    fn default() -> Depth {
        Depth {
            imbed: 0.0,
            mask: false,
            inner: -1.0,
            outer: 1.0,
            infinite: false,
            axes: [false; 3],
            curve: Curve::constant(1.0),
        }
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    clippy::float_cmp,
    reason = "serde's skip_serializing_if passes a reference; the default is the exact -1"
)]
fn is_minus_one(x: &f32) -> bool {
    *x == -1.0
}
#[expect(
    clippy::trivially_copy_pass_by_ref,
    clippy::float_cmp,
    reason = "serde's skip_serializing_if passes a reference; the default is the exact 1"
)]
fn is_one(x: &f32) -> bool {
    *x == 1.0
}
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn no_axis(axes: &[bool; 3]) -> bool {
    !axes.contains(&true)
}
fn is_flat(c: &Curve) -> bool {
    *c == Curve::constant(1.0)
}

/// The tip: falloff curve, hardness, Falloff and alpha.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct TipSpec {
    pub profile: ProfileName,
    /// The curve of a `custom` profile, distance (0 center, 1 rim) to strength.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Curve>,
    /// 0 to 0.99: full strength out to this fraction of the radius.
    pub hardness: f32,
    /// The Falloff setting, -100 (wide) to 100 (narrow).
    pub focal: f32,
    /// Wrap Mode, 1 (off) to 16: the stroke repeated over that many tiles along each side of
    /// the part's plane (decision 0054). Left out of the preset's text at 1, so a preset without
    /// it keeps its hash.
    #[serde(skip_serializing_if = "is_one_tile")]
    pub wrap: u32,
    /// Tilt Brush, -90 to 90 degrees: each pass's dab frame leans its normal toward its first
    /// axis by this angle (decision 0058). Left out of the text at 0, as are the next two at
    /// their defaults.
    #[serde(skip_serializing_if = "is_zero")]
    pub tilt: f32,
    /// Constant Tilt: every dab's first axis is the stroke's first step, laid across its surface.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub constant_tilt: bool,
    /// Align To Path, 0 to 100: at 100 a dab's first axis is its step; lower, it turns toward
    /// the step over a length of stroke that grows to a brush diameter at 0.
    #[serde(skip_serializing_if = "is_hundred")]
    pub align: f32,
    /// Accurate Curve: a custom profile and the Zero Curve are read exactly, not through their
    /// tables (decision 0059). Left out of the text when off, as are the next three at their
    /// defaults.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub accurate_curve: bool,
    /// Curve By Pen: pressure blends the falloff from the Zero Curve (no pressure) to the
    /// profile, the Edit Curve (full pressure), by the Pen Curve.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub curve_by_pen: bool,
    /// The Zero Curve, distance (0 center, 1 rim) to strength, values 0 to 1: `1 - d` unless set.
    #[serde(skip_serializing_if = "is_falling")]
    pub zero_curve: Curve,
    /// The Pen Curve, pressure to the share of the Edit Curve, values 0 to 1: `y = x` unless set.
    #[serde(skip_serializing_if = "Curve::is_identity")]
    pub pen_curve: Curve,
    /// The alpha and how the brush uses it; none for a plain round tip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha: Option<AlphaSpec>,
}

impl Default for TipSpec {
    fn default() -> TipSpec {
        TipSpec {
            profile: ProfileName::Smooth,
            points: None,
            hardness: 0.0,
            focal: 0.0,
            wrap: 1,
            tilt: 0.0,
            constant_tilt: false,
            align: 100.0,
            accurate_curve: false,
            curve_by_pen: false,
            zero_curve: Curve::linear().inverted(),
            pen_curve: Curve::linear(),
            alpha: None,
        }
    }
}

fn is_falling(c: &Curve) -> bool {
    *c == Curve::linear().inverted()
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_one_tile(n: &u32) -> bool {
    *n == 1
}
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_zero(x: &f32) -> bool {
    *x == 0.0
}
#[expect(
    clippy::trivially_copy_pass_by_ref,
    clippy::float_cmp,
    reason = "serde's skip_serializing_if passes a reference; the default is the exact 100"
)]
fn is_hundred(x: &f32) -> bool {
    *x == 100.0
}

impl TipSpec {
    /// The falloff this tip describes.
    pub fn tip(&self) -> Tip {
        let profile = match (self.profile, &self.points) {
            (ProfileName::Custom, Some(c)) => Profile::Custom(c.clone()),
            (ProfileName::Sphere, _) => Profile::Sphere,
            (ProfileName::Sharp, _) => Profile::Sharp,
            (ProfileName::Linear, _) => Profile::Linear,
            (ProfileName::Constant, _) => Profile::Constant,
            (ProfileName::Soft, _) => Profile::Soft,
            (ProfileName::Hard, _) => Profile::Hard,
            (ProfileName::FlatTop, _) => Profile::FlatTop,
            (ProfileName::SharpCenter, _) => Profile::SharpCenter,
            _ => Profile::Smooth,
        };
        let mut tip = Tip::new(profile, self.hardness, self.focal);
        if self.curve_by_pen {
            tip = tip.by_pen(&self.zero_curve);
        }
        if self.accurate_curve {
            tip = tip.accurate();
        }
        tip
    }

    /// The share of the Edit Curve a dab at `pressure` takes: the Pen Curve there with Curve
    /// By Pen on, all of it with it off (decision 0059).
    pub fn share(&self, pressure: f32) -> f32 {
        if self.curve_by_pen {
            self.pen_curve.eval(pressure)
        } else {
            1.0
        }
    }
}

/// The tip's profile: the named ones (soft, hard, flat top and sharp center are the shipped
/// presets of decision 0059), or `custom` points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileName {
    Smooth,
    Sphere,
    Sharp,
    Linear,
    Constant,
    Soft,
    Hard,
    FlatTop,
    SharpCenter,
    Custom,
}

/// How the dab frame reads the surface.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Sampling {
    /// Radius of the area normal, as a fraction of the brush radius.
    pub normal_radius: f32,
    /// Radius of the area center, as a fraction of the brush radius.
    pub position_radius: f32,
    /// Read the surface with this stroke's changes (Buildup), or as the stroke started.
    pub accumulate: bool,
    /// Only vertices facing the camera count.
    pub front_only: bool,
}

impl Default for Sampling {
    fn default() -> Sampling {
        Sampling {
            normal_radius: 0.5,
            position_radius: 0.5,
            accumulate: true,
            front_only: true,
        }
    }
}

/// Pen pressure curves: pressure to a multiplier.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Pressure {
    pub size: Curve,
    pub strength: Curve,
    pub spacing: Curve,
    /// Multiplies the Brush Modifier.
    pub modifier: Curve,
    /// Multiplies `depth.imbed`.
    pub imbed: Curve,
    /// Stroke with the session's global pressure curves instead. The engine writes them into
    /// the stroke's settings, so a stroke still names every curve it uses. Without it a brush
    /// still takes the session's size curve, unless its file names a `size` of its own
    /// (decision 0036).
    pub global: bool,
}

impl Default for Pressure {
    /// Pressure scales strength; size, spacing, the Brush Modifier and imbed ignore it.
    fn default() -> Pressure {
        Pressure {
            size: Curve::constant(1.0),
            strength: Curve::linear(),
            spacing: Curve::constant(1.0),
            modifier: Curve::constant(1.0),
            imbed: Curve::constant(1.0),
            global: false,
        }
    }
}

/// Effects layered on any kernel, every dab.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Post {
    /// Gain of a Smooth pass after the kernel.
    pub auto_smooth: f32,
    /// The Smooth mode of that pass: `relax` evens out edge lengths (Move Elastic).
    pub auto_smooth_mode: SmoothMode,
    /// The pass is skipped on parts with more faces than this, because its cost grows with
    /// density; 0 never skips.
    pub auto_smooth_max_faces: u32,
    /// Pull along `gravity_dir`, -1 to 1.
    pub gravity: f32,
    pub gravity_dir: [f32; 3],
    /// Topology Rake, not built yet: must stay 0.
    pub topology_rake: f32,
}

impl Default for Post {
    fn default() -> Post {
        Post {
            auto_smooth: 0.0,
            auto_smooth_mode: SmoothMode::Uniform,
            auto_smooth_max_faces: 0,
            gravity: 0.0,
            gravity_dir: [0.0, -1.0, 0.0],
            topology_rake: 0.0,
        }
    }
}

/// Temporary masks computed while the brush works.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Automask {
    /// 0 to 1: how much vertices facing away from the camera are spared.
    pub backface: f32,
    /// -1 to 1: positive spares hollows, negative spares peaks.
    pub cavity: f32,
    /// Only vertices joined to the stroke start along edges within this many radii; 0 off.
    pub topology: f32,
    /// 0 to 1: how much faces outside the face group under the stroke start are spared.
    pub face_group: f32,
}
