//! A view's navigation settings: what it orbits about, where it turns, how its moves behave, and
//! how `session.state` names them.

use gs_core::camera::{Axis, OrbitMode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Views;
use crate::view::spec::ProjectionArg;

/// What a horizontal drag turns the view about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OrbitArg {
    /// The screen's vertical axis: the view turns relative to the screen.
    Free,
    /// The scene's X axis, like a sculpture stand lying on its side.
    X,
    /// The scene's Y axis, the scene's up: a turntable, with the horizon kept level (the
    /// default, decision 0017 as amended).
    #[default]
    Y,
    /// The scene's Z axis.
    Z,
}

impl From<OrbitArg> for OrbitMode {
    fn from(o: OrbitArg) -> Self {
        match o {
            OrbitArg::Free => OrbitMode::Free,
            OrbitArg::X => OrbitMode::Axis(Axis::X),
            OrbitArg::Y => OrbitMode::Axis(Axis::Y),
            OrbitArg::Z => OrbitMode::Axis(Axis::Z),
        }
    }
}

/// Where a view turns and zooms about when no point is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PivotRule {
    /// The point the camera looks at, the middle of the picture, which the wheel and panning
    /// move (the default, decision 0017 as amended).
    #[default]
    Target,
    /// Where the last stroke first touched the surface; after a frame, or before any stroke,
    /// the point the camera looks at.
    LastEdit,
    /// The centre of every part.
    Part,
    /// The surface under the press that starts the move (`at`), else as `last-edit`.
    UnderPointer,
}

/// How a view's moves behave beyond what they turn about (`view.settings`, decision 0017 as
/// amended). The defaults leave every move as it was before they existed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, JsonSchema)]
pub struct NavSettings {
    /// Recenter Rate (canvas.navigation): the share of the way to the middle of the picture an
    /// orbit slides the pivot per 90 degrees of turn, 0 to 1. 0, the default, keeps the pivot
    /// where it is on the screen.
    pub recenter_rate: f32,
    /// The step a snapping orbit lands on, in whole degrees dividing 90. 90, the default, lands
    /// on the six axis views.
    pub snap_deg: u32,
    /// Auto Adjust Distance (canvas.perspective): on, the default, a zoom changes the camera's
    /// distance, so the perspective strengthens as it comes close; off, in perspective, a zoom
    /// slides the camera keeping its distance, and it may pass through the model. On whatever
    /// this says while the pivot rule is `last-edit` (canvas.local-pivot).
    pub auto_distance: bool,
    /// Align To Object (canvas.perspective): on, a pan in perspective slides the picture by the
    /// lens shift, so the model is seen from the same direction anywhere on the screen; off, the
    /// default, a pan slides the camera. No effect in orthographic.
    pub align_to_object: bool,
    /// Right-drag moves the pivot (canvas.local-pivot): on, the default, `view.touch` moves the
    /// last-edit point to the surface under the pixel; off, it changes nothing.
    pub right_drag_pivot: bool,
}

impl Default for NavSettings {
    fn default() -> Self {
        Self {
            recenter_rate: 0.0,
            snap_deg: 90,
            auto_distance: true,
            align_to_object: false,
            right_drag_pivot: true,
        }
    }
}

/// A view's settings, as `session.state` names them for `main`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MainSettings {
    pub locked: bool,
    pub orbit: OrbitArg,
    pub pivot: PivotRule,
    pub projection: ProjectionArg,
    pub wireframe: bool,
}

impl Views {
    /// View `name`'s lock, orbit mode, pivot rule, projection and wireframe, for
    /// `session.state`.
    pub(crate) fn settings_of(&self, name: &str) -> MainSettings {
        let view = self.view(name);
        MainSettings {
            locked: view.locked,
            orbit: view.orbit,
            pivot: view.pivot,
            projection: (view.camera).map_or(ProjectionArg::Perspective, |c| c.projection.into()),
            wireframe: self.wireframe(name),
        }
    }
}
