//! The kernel a preset runs and its settings: the only brush-specific step of a dab. What each
//! kernel does is specified in `crate::brush` ("Kernels"); `crate::kernels` runs them.

use serde::{Deserialize, Serialize};

use super::BrushPreset;
use crate::columns::AttrDef;
use crate::math::Vec3;

/// The kernel and its settings: the only brush-specific step of a dab.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KernelSpec {
    Offset(OffsetSpec),
    Plane(PlaneSpec),
    Smooth(SmoothSpec),
    Move(MoveSpec),
    Hook(HookSpec),
    Attribute(AttrSpec),
}

impl KernelSpec {
    /// Its name, as preset files write `kernel.type`.
    pub fn name(&self) -> &'static str {
        match self {
            KernelSpec::Offset(_) => "offset",
            KernelSpec::Plane(_) => "plane",
            KernelSpec::Smooth(_) => "smooth",
            KernelSpec::Move(_) => "move",
            KernelSpec::Hook(_) => "hook",
            KernelSpec::Attribute(_) => "attribute",
        }
    }

    /// What the Brush Modifier does on this kernel, in words; `None` where it has no meaning
    /// yet (decision 0018, choice 3; on Hook, decision 0049, section 7).
    pub fn modifier_meaning(&self) -> Option<&'static str> {
        match self {
            KernelSpec::Offset(_) => Some(
                "positive pinches toward the stroke line, negative inflates along each vertex \
                 normal",
            ),
            KernelSpec::Hook(_) => Some(
                "on a screen stroke, positive pulls more along the start normal, negative more \
                 across the screen",
            ),
            _ => None,
        }
    }

    /// The term the Brush Modifier `m` (-1 to 1, after its pressure curve) adds to an Offset
    /// kernel: a pinch for positive `m`, an inflate for negative, nothing at 0.
    pub fn modifier_term(m: f32) -> Option<OffsetTerm> {
        if m > 0.0 {
            Some(OffsetTerm {
                dir: Direction::ToCenter,
                gain: 2.0 * m,
                exponent: 1.0,
                tangent: true,
            })
        } else if m < 0.0 {
            Some(OffsetTerm {
                dir: Direction::VertexNormal,
                gain: -0.5 * m,
                exponent: 1.0,
                tangent: false,
            })
        } else {
            None
        }
    }

    /// True when an alpha's factor keeps its sign (`v - mid`, so it can carve) on this kernel;
    /// otherwise the factor is an intensity from 0 to 1 (decision 0018, choice 2).
    pub fn signed_alpha(&self) -> bool {
        matches!(self, KernelSpec::Offset(_))
    }
}

/// Push along chosen directions.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OffsetSpec {
    /// One or two terms, summed.
    pub terms: Vec<OffsetTerm>,
}

/// Pull toward a plane fitted under the brush.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlaneSpec {
    #[serde(default)]
    pub source: PlaneSource,
    #[serde(default)]
    pub side: Side,
    pub gain: f32,
    /// Highest vertex above the plane that moves, in radii.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    /// Deepest vertex below the plane that moves, in radii.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f32>,
}

/// Pull toward a neighbour average.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SmoothSpec {
    pub mode: SmoothMode,
    pub gain: f32,
    /// Weight Strength: how far `group_borders` pulls a point toward its border neighbours'
    /// mean, 0 to 1 (decision 0049, section 3). No other mode reads it.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub weight: f32,
    /// Min Connected Points: a point moves only if it averages at least this many neighbours,
    /// 1 to 64 (section 5).
    #[serde(default = "one_point", skip_serializing_if = "is_one_point")]
    pub min_connected: u32,
    /// Projection: each move loses its part along the point's normal at the stroke's start
    /// (section 4).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub projection: bool,
}

impl SmoothSpec {
    /// `mode` at `gain`, every other setting at its default.
    pub fn new(mode: SmoothMode, gain: f32) -> SmoothSpec {
        SmoothSpec {
            mode,
            gain,
            weight: 1.0,
            min_connected: 1,
            projection: false,
        }
    }

    /// Whether it runs exactly the code the first four modes ran: one of them, Min Connected
    /// Points at most 1 and Projection off (decision 0049, section 6).
    pub fn plain(&self) -> bool {
        self.mode.old() && self.min_connected <= 1 && !self.projection
    }
}

/// Grab at stroke start and drag.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MoveSpec {
    pub select: Select,
    /// Elastic only: where the move fades out, in radii.
    #[serde(default = "four")]
    pub support: f32,
    /// Elastic only: Poisson ratio, 0 to 0.5 (0.5 keeps volume).
    #[serde(default = "poisson")]
    pub poisson: f32,
}

/// Move that grabs again at every dab (Pull Out).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HookSpec {
    /// Pull toward the line of motion, 0 to 1.
    #[serde(default)]
    pub pinch: f32,
    /// Where a screen stroke pulls (decision 0049, section 7).
    #[serde(default, skip_serializing_if = "PullDirection::is_screen")]
    pub direction: PullDirection,
}

/// Where a Hook stroke through a camera pulls: the plane its later samples land on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullDirection {
    /// Across the screen: the plane through the grab facing the camera.
    #[default]
    Screen,
    /// Out along the stroke's start normal: the plane through the grab holding that normal
    /// and facing the camera most.
    Normal,
}

impl PullDirection {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn is_screen(&self) -> bool {
        *self == PullDirection::Screen
    }
}

impl BrushPreset {
    /// The normal of the plane a screen stroke's later samples land on, for a Hook kernel at
    /// pen pressure `pressure`, seen along `view`, with the stroke's start area normal `start`
    /// (decision 0049, section 7): `normalize((1 - b) view + b u)`, with `u` the part of `view`
    /// across `start`, normalized, and `b` the blend of the direction and the Brush Modifier.
    /// `None` means the plane facing the camera: another kernel, no start yet, a blend of 0, or
    /// a start normal along the view.
    pub fn pull_plane(&self, view: Vec3, pressure: f32, start: Option<Vec3>) -> Option<Vec3> {
        let (KernelSpec::Hook(k), Some(n)) = (&self.kernel, start) else {
            return None;
        };
        let base = match k.direction {
            PullDirection::Screen => 0.0,
            PullDirection::Normal => 1.0,
        };
        let m = self.modifier / 100.0 * self.pressure.modifier.eval(pressure);
        let b = (base + m).clamp(0.0, 1.0);
        let u = view - n * n.dot(view);
        if b == 0.0 || u.len2() < 1e-6 {
            return None;
        }
        Some((view * (1.0 - b) + u.normalized() * b).normalized())
    }
}

/// Write a value into an attribute column.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttrSpec {
    pub column: Column,
    #[serde(default)]
    pub op: AttrOp,
    /// Mask 0 to 1, a UV hint -1 to 1, a face group id, a packed RGBA color, or hide 1.
    #[serde(default)]
    pub value: f64,
    pub gain: f32,
}

fn four() -> f32 {
    4.0
}
fn poisson() -> f32 {
    0.45
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
fn one_point() -> u32 {
    1
}
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_one_point(n: &u32) -> bool {
    *n == 1
}

/// One Offset term.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OffsetTerm {
    pub dir: Direction,
    pub gain: f32,
    /// Power on the influence: higher concentrates the term at the center.
    #[serde(default = "one")]
    pub exponent: f32,
    /// `to_center` only: pull within the surface, not into it.
    #[serde(default)]
    pub tangent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    AreaNormal,
    VertexNormal,
    ToCenter,
    View,
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaneSource {
    /// Fitted again at every dab, so it follows the form.
    #[default]
    Dab,
    /// Fitted at the first dab and kept for the stroke.
    Stroke,
    /// The first dab's normal kept for the stroke, the center read at every dab: the plane
    /// keeps its tilt and follows the height of the form.
    StrokeNormal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    #[default]
    Both,
    Above,
    Below,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SmoothMode {
    #[default]
    Uniform,
    Stronger,
    Tangent,
    Relax,
    /// Only along the stroke's direction (decision 0049, section 2).
    AlongStroke,
    /// Only across it (the same section).
    AcrossStroke,
    /// Keeping face group borders (section 3).
    GroupBorders,
}

impl SmoothMode {
    /// One of the four modes built before decision 0049.
    pub fn old(self) -> bool {
        matches!(
            self,
            SmoothMode::Uniform | SmoothMode::Stronger | SmoothMode::Tangent | SmoothMode::Relax
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Select {
    Ball,
    Connected,
    Elastic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    Mask,
    Group,
    Color,
    Hide,
    /// Where Auto UV puts seams, -1 to 1 (decision 0061, section 6).
    UvHint,
    /// How dense Auto UV's texels are, -1 to 1.
    UvDensity,
}

impl Column {
    /// The column a brush on it writes.
    pub fn def(self) -> AttrDef {
        match self {
            Column::Mask => AttrDef::mask(),
            Column::Group => AttrDef::face_group(),
            Column::Color => AttrDef::color(),
            Column::Hide => AttrDef::hide(),
            Column::UvHint => AttrDef::uv_hint(),
            Column::UvDensity => AttrDef::uv_density(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttrOp {
    #[default]
    Set,
    Blur,
    Sharpen,
}
