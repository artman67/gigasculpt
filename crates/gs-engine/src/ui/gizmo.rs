//! `ui.gizmo`: the window's gizmo tool (decision 0027, section 11). While it is on, the 3D view
//! shows the gizmo of the first part that shows, and the pen drives it instead of sculpting.
//! The window keeps the settings with its layout state, as it keeps navigation; the size and
//! the screen's pixels per point are the ones `gizmo.describe` draws with.
//!
//! The window draws and picks the gizmo from the session, read through the two helpers here:
//! the pose of the gizmo it shows, and the surface under a press. It shows the gizmo of the
//! part a press last put it on, or else of the first part that shows.

use gs_core::Vec3;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::no_window;
use crate::{Command, Ctx, Example, errors, register_command};

/// The gizmo's smallest and largest size on screen, in points (gs-core's `Style`).
const SIZES: [f32; 2] = [10.0, 200.0];

/// The window's gizmo tool and its settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GizmoSettings {
    /// The tool is on: the 3D view shows the gizmo, and the pen drags its handles, puts it
    /// where it presses on the model, and navigates on empty canvas.
    pub tool: bool,
    /// The gizmo's size on screen, in points.
    pub size: f32,
    /// While Shift is held a turn snaps to whole steps of a full turn: 72 is 5 degrees.
    pub snap: u32,
    /// The mask holds points back in a drag (`transform.apply`'s `mask`).
    pub mask: bool,
    /// A drag's falloff, -100 (rigid: the whole part moves) to 100 (`transform.apply`'s
    /// `falloff`). Above -100 it reaches as far as the brush's Size at the gizmo.
    pub falloff: f32,
    /// The part whose gizmo the window shows: the one a press last put it on while it shows,
    /// else the first part that shows. `None` with the tool off or no part showing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

/// Turn the window's gizmo tool on or off, or change its size on screen, how a turn snaps
/// while Shift is held, whether the mask holds points back in a drag and a drag's falloff. Left
/// out, a setting stays. Answers with every setting and the part whose gizmo shows.
///
/// With the tool on, a pen press on a handle drags it: a locked gizmo transforms the part
/// (`gizmo.drag`, logged as one `transform.apply` when the pen lifts, then one `gizmo.place`
/// that follows what moved unless the gizmo is sticky); an unlocked one, or any with Alt held,
/// only moves the gizmo (one `gizmo.place` when the pen lifts). A press on the model puts the
/// gizmo there, along the surface (`gizmo.place`); a press on empty canvas navigates. Esc or
/// the barrel button cancels a drag. A drag's falloff is the gizmo's own (-100, rigid, unless
/// changed), and above -100 it reaches as far as the brush's Size at the gizmo; it mirrors
/// across the brush's mirror planes. With the brush's radial symmetry on, a press that would
/// transform the part is refused, saying why on the status line, as `transform.apply` refuses
/// radial symmetry. The settings are kept with the window's layout state; not logged.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiGizmo {
    /// True: the pen drives the gizmo instead of sculpting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<bool>,
    /// The gizmo's size on screen, in points, 10 to 200.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 10.0, max = 200.0))]
    pub size: Option<f32>,
    /// Steps of a full turn a Shift drag snaps to, 1 to 3600: 72 is 5 degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 3600))]
    pub snap: Option<u32>,
    /// True: the mask holds points back in a drag, as `transform.apply`'s `mask`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<bool>,
    /// A drag's falloff, -100 (rigid: the whole part moves) to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = -100.0, max = 100.0))]
    pub falloff: Option<f32>,
}

impl Command for UiGizmo {
    const NAME: &'static str = "ui.gizmo";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes how the window reads the pen, not the session: the drags and placements it
    /// leads to are logged themselves.
    const LOGGED: bool = false;
    type Output = GizmoSettings;

    fn examples() -> Vec<Example<Self>> {
        let on = Self {
            tool: Some(true),
            ..Self::default()
        };
        vec![
            Example::new("the pen drives the gizmo (needs the window)", on)
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<GizmoSettings, CommandError> {
        if self
            .size
            .is_some_and(|s| !(SIZES[0]..=SIZES[1]).contains(&s))
        {
            return Err(errors::args_invalid("/size", "must be from 10 to 200"));
        }
        if self.snap.is_some_and(|s| !(1..=3600).contains(&s)) {
            return Err(errors::args_invalid("/snap", "must be from 1 to 3600"));
        }
        if self.falloff.is_some_and(|f| !(-100.0..=100.0).contains(&f)) {
            return Err(errors::args_invalid("/falloff", "must be from -100 to 100"));
        }
        (cx.window.with(|w| w.gizmo(Some(&self)))).ok_or_else(no_window)?;
        show(cx);
        // Read again: the part shown is the one `show` settled on.
        (cx.window.with(|w| w.gizmo(None))).ok_or_else(no_window)
    }
}
register_command!(UiGizmo, "Gizmo tool");

/// Hands the window the pose of the gizmo it shows, if it shows one. Every window command that
/// draws or picks the gizmo calls it first, so the window never works from a pose older than
/// the call.
pub(crate) fn show(cx: &Ctx) {
    let pose = |part: &str| crate::gizmo::pose_of(cx, part).ok().map(|(pose, _)| pose);
    cx.window.with(|w| w.show_gizmo(&pose));
}

/// The first surface the ray from `o` along unit `d` hits among the parts that show: the part,
/// the scene point and its normal. The window's press on the model, when no handle is under
/// it. The search is `pick.ray`'s (`pick::first_hit`).
pub(crate) fn under(cx: &Ctx, o: Vec3, d: Vec3) -> Option<Surface> {
    let doc = cx.doc.document();
    let parts: Vec<_> = (doc.shown()).map(|(n, p)| (n.as_str(), p)).collect();
    let (_, hit, part, p) = crate::pick::first_hit(&parts, o, d)?;
    let t = p.transform();
    let corner = crate::pick::nearest_corner(p.store(), hit.face, hit.pos);
    Some(Surface {
        part: part.to_string(),
        point: t.point(hit.pos),
        normal: t.direction(hit.normal),
        corner: t.point(gs_core::PosRead::get(&p.store().pos(), corner)),
        color: crate::pick::paint_at(p.store(), hit.face, hit.pos),
    })
}

/// A point on a part's surface, the surface's normal there, the corner of the face hit
/// nearest the point (where Alt + click snaps the gizmo), and the color stored there, as
/// `pick.color` answers it (the window's color pick).
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub part: String,
    pub point: Vec3,
    pub normal: Vec3,
    pub corner: Vec3,
    pub color: [u8; 3],
}
