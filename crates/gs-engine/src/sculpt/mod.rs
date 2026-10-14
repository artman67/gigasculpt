//! Sculpting: `brush.stroke` and the brushes it runs (`brush.list`, `brush.describe`), and the
//! session's brush settings the window strokes with (`brush.set`).
//!
//! A stroke runs gs-core's stroke pipeline inside the part's write guard, so it is one undo step
//! and one log line like any edit. Samples come in world space, or in screen space with a
//! camera: then each one is aimed at the part through the camera (see `aim`). The window's pen
//! takes the same path one sample at a time while the pen is down (see `live`), and commits the
//! stroke as the `brush.stroke` call that replays it.
//!
//! A brush in use is its preset with a patch, the settings changed for it (decision 0018): the
//! session keeps one per brush (`settings`), a stroke carries its own, and gs-core resolves
//! them (`patch`).

mod aim;
mod brush_file;
mod brushes;
mod colors;
mod curve_file;
mod detail;
mod detail_pick;
mod live;
mod patch;
mod pen;
mod placed;
pub(crate) mod presets;
mod settings;
mod slots;
mod steps;
mod stroke;
mod symmetry;

use gs_core::symmetry::Symmetry;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) use brushes::about;
pub use brushes::{BrushInfo, brushes};
pub use colors::Colors;
pub(crate) use colors::word;
pub(crate) use curve_file::curve_schema;
pub(crate) use detail::blocked_on_active;
pub(crate) use detail::torn_uvs;
pub use detail::{Blocked, DetailMode, DetailSettings, DetailState, Refusal};
pub(crate) use live::Live;
pub use live::{PenDown, PenStroke};
pub use pen::PenSettings;
pub use settings::BrushState;
pub(crate) use settings::{BrushSet, BrushSettings};
pub use slots::{Slot, Slots};
pub use steps::Increments;
pub(crate) use stroke::{aim_camera, carry_presets};
pub use symmetry::SymmetrySettings;

/// The session's sculpting state: the brush settings the window strokes with, and the pen
/// stroke in progress.
#[derive(Default)]
pub(crate) struct Sculpting {
    pub settings: BrushSettings,
    pub live: Option<Live>,
    /// Why the last pen stroke was refused, for the window's status bar.
    pub problem: Option<String>,
    /// The pen's last press landed on empty canvas, so the window navigates with it.
    pub on_canvas: bool,
    /// The preset files strokes carried in full, by hash (decision 0042): a later stroke that
    /// pins one of these hashes sculpts with it when today's brush differs.
    pub recorded: std::collections::BTreeMap<String, String>,
}

/// A choice of the three axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Axes {
    #[serde(default)]
    pub x: bool,
    #[serde(default)]
    pub y: bool,
    #[serde(default)]
    pub z: bool,
}

impl Axes {
    fn array(self) -> [bool; 3] {
        [self.x, self.y, self.z]
    }

    /// No axis is chosen.
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's skip_serializing_if passes a reference"
    )]
    fn none(&self) -> bool {
        *self == Axes::default()
    }

    /// Axis `a`'s choice.
    fn at(&mut self, a: Axis) -> &mut bool {
        match a {
            Axis::X => &mut self.x,
            Axis::Y => &mut self.y,
            Axis::Z => &mut self.z,
        }
    }
}

/// An axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
    Z,
}

/// Copies of every dab around an axis through the part's origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Radial {
    pub axis: Axis,
    /// How many copies, 2 to 64.
    #[schemars(range(min = 2, max = 64))]
    pub count: u32,
}

/// Symmetry: every dab is repeated mirrored across the chosen planes through the part's
/// origin (`x` mirrors across the plane x = 0), and around an axis for radial symmetry.
/// Overlapping copies near a plane are feathered so they never add up to more than one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymmetryArg {
    /// Mirror planes.
    #[serde(default)]
    pub mirror: Axes,
    /// Parallel planes: the dab repeats at the place mirrored across the plane but goes the
    /// way the stroke goes, so a Move dragged toward -X moves its copy toward -X too, and an
    /// alpha reads the same way round on both sides (decision 0044). A plane is Mirror or
    /// Parallel, not both.
    #[serde(default, skip_serializing_if = "Axes::none")]
    pub parallel: Axes,
    /// Radial copies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radial: Option<Radial>,
}

impl SymmetryArg {
    /// gs-core's symmetry: the window draws a select outline's copies through its passes.
    pub fn core(self) -> Symmetry {
        Symmetry {
            mirror: self.mirror.array(),
            parallel: self.parallel.array(),
            radial: self.radial.map(|r| (r.axis as usize, r.count)),
            ..Symmetry::default()
        }
    }

    /// [`SymmetryArg::core`], refusing a plane that is both Mirror and Parallel.
    pub(crate) fn checked(self) -> Result<Symmetry, gs_protocol::CommandError> {
        let (m, p) = (self.mirror.array(), self.parallel.array());
        if (0..3).any(|a| m[a] && p[a]) {
            let message = "a plane is Mirror or Parallel, not both";
            return Err(crate::errors::args_invalid("/symmetry/parallel", message));
        }
        Ok(self.core())
    }
}
