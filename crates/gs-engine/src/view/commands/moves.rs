//! The camera moves: `view.orbit`, `view.roll`, `view.pan` and `view.dolly`. Each turns or
//! scales the camera about a point that keeps its place on the screen (gs-core
//! `camera::nav`), so orbiting about the pivot never makes the work jump.

use gs_core::camera::OrbitMode;
use gs_core::camera::Projection::Perspective;
use gs_protocol::{CommandError, Cost, Kind, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::spec::ViewName;
use super::super::views::{OrbitArg, PivotRule};
use super::{Change, Moved, Touches, change, may_change, name};
use crate::command::ViewArg;
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// Turn a view about its pivot: the camera moves `right_deg` to its right (the model seems to
/// turn left) and `up_deg` upward (it looks more from above).
///
/// The horizontal turn is about a scene axis (`x`, `y`, `z`; `y`, the default, is a turntable
/// that keeps the horizon level) or the screen's vertical axis (`free`); the vertical turn is
/// always about the screen's horizontal axis. About a scene axis, a turn that starts with the
/// camera upside down goes sideways the other way, so the model follows the pointer; a gesture
/// keeps the way it started with. The pivot keeps its place on the screen, unless the view's
/// Recenter Rate (`view.settings`) slides it toward the middle as the view turns. One step of
/// the view's camera history, or one call of a `gesture`; never part of document undo.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewOrbit {
    /// The view to turn: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// Degrees the camera moves to its right about the pivot.
    #[serde(default)]
    pub right_deg: f32,
    /// Degrees the camera moves upward about the pivot.
    #[serde(default)]
    pub up_deg: f32,
    /// What the horizontal turn is about. Left out: the view's setting (`view.settings`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<OrbitArg>,
    /// Land on the nearest of the six axis views, with a level horizon (with a snap step under
    /// 90 degrees in `view.settings`, the nearest view at whole steps of turn and height). In a
    /// gesture the turn under the snap goes on, and a call without it keeps the snapped view.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub snap: bool,
    /// The point to turn about, in scene units. Left out: the view's pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<Vec3>,
    /// The pixel the move starts at, `[x, y]`, for the `under-pointer` pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
    /// Calls with the same id on the same view are one gesture: one history step and one log
    /// line, the camera it ended on. The pivot, and whether it turns sideways the other way,
    /// stay as the gesture started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<u64>,
    /// This call ends the gesture.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
}

/// Roll a view about its direction through the pivot: positive turns the picture
/// counterclockwise. One step of the view's camera history, or one call of a `gesture`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewRoll {
    /// The view to roll: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// Degrees, counterclockwise on the picture.
    pub deg: f32,
    /// The point to roll about, in scene units. Left out: the view's pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<Vec3>,
    /// The pixel the move starts at, for the `under-pointer` pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
    /// Calls with the same id on the same view are one gesture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<u64>,
    /// This call ends the gesture.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
}

/// Slide a view parallel to the screen by pixels, so the pivot's image moves exactly `dx` to
/// the right and `dy` down: the model follows the pointer. With the view's Align To Object
/// setting on (`view.settings`), a perspective view slides its picture by the lens shift
/// instead, so the model shows the same side anywhere on the screen. One step of the view's
/// camera history, or one call of a `gesture`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewPan {
    /// The view to slide: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// Pixels to the right.
    #[serde(default)]
    pub dx: f32,
    /// Pixels down.
    #[serde(default)]
    pub dy: f32,
    /// The point whose image follows the pointer. Left out: the view's pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<Vec3>,
    /// The pixel the move starts at, for the `under-pointer` pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
    /// Calls with the same id on the same view are one gesture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<u64>,
    /// This call ends the gesture.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
}

/// Move a view closer or farther: `factor` below 1 comes closer (0.5 halves the distance), above
/// 1 backs away. The pivot, or with `toward` the surface point under that pixel, keeps its
/// place on the screen. With the view's Auto Adjust Distance setting off (`view.settings`), a
/// perspective view moves that way by as much but keeps its distance, so the perspective stays
/// and the camera may pass through the model. One step of the view's camera history, or one
/// call of a `gesture`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewDolly {
    /// The view to move: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// The distance after over the distance before.
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub factor: f32,
    /// A pixel `[x, y]`: move toward the surface under it (or the pivot's depth along its ray
    /// on empty canvas). Left out: toward the pivot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toward: Option<[f32; 2]>,
    /// The point that keeps its place. Left out: the view's pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<Vec3>,
    /// The pixel the move starts at, for the `under-pointer` pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
    /// Calls with the same id on the same view are one gesture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<u64>,
    /// This call ends the gesture.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub end: bool,
}

/// Every number is finite, or `E_ARGS_INVALID` at the first that is not.
fn finite(values: &[(&str, f32)]) -> Result<(), CommandError> {
    match values.iter().find(|(_, v)| !v.is_finite()) {
        Some((path, _)) => Err(errors::args_invalid(path, "must be a finite number")),
        None => Ok(()),
    }
}

impl Command for ViewOrbit {
    const NAME: &'static str = "view.orbit";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Pivot;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        let sphere = json!({});
        vec![
            Example::raw("turn 30 degrees to the right", json!({ "right_deg": 30 }))
                .after("primitive.sphere", sphere.clone()),
            Example::raw(
                "a free turn about the screen's axes, then up 20 degrees",
                json!({ "right_deg": 45, "up_deg": 20, "mode": "free" }),
            )
            .after("primitive.sphere", sphere.clone()),
            Example::raw(
                "turn a little and snap to the nearest side",
                json!({ "right_deg": 80, "snap": true }),
            )
            .after("primitive.sphere", sphere),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        finite(&[("/right_deg", self.right_deg), ("/up_deg", self.up_deg)])?;
        let view = name(self.view.as_ref())?.to_string();
        let mode = OrbitMode::from(self.mode.unwrap_or(cx.views.get(&view).orbit));
        let nav = cx.views.get(&view).nav;
        let (right, up) = (self.right_deg.to_radians(), self.up_deg.to_radians());
        let turn = (right * right + up * up).sqrt();
        let step = (nav.snap_deg as f32).to_radians();
        let c = Change {
            view: self.view.as_ref(),
            viewport: self.viewport,
            pivot: self.pivot,
            gesture: self.gesture,
            end: self.end,
            remember: true,
            moves: Touches::TURN,
        };
        let snap = self.snap;
        change(cx, &c, |camera, pivot, views, _| {
            let flip = views.flipped(&view, camera.upside_down(mode));
            let right = if flip { -right } else { right };
            if snap {
                // The turn goes on under the snap: orbit the held turn, show its snapped view.
                let mut free = *camera;
                free.turn_to(views.held_turn(&view).unwrap_or(camera.orientation), pivot);
                free.orbit(pivot, mode, right, up);
                views.hold_turn(&view, Some(free.orientation));
                *camera = free;
                camera.turn_to(free.snapped_by(step), pivot);
            } else {
                views.hold_turn(&view, None);
                camera.orbit(pivot, mode, right, up);
            }
            camera.recenter(pivot, nav.recenter_rate, turn);
        })
    }
}
register_command!(ViewOrbit, "Orbit");

impl Command for ViewRoll {
    const NAME: &'static str = "view.roll";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Pivot;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "roll the picture 90 degrees counterclockwise",
                json!({ "deg": 90 }),
            )
            .after("primitive.cube", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        finite(&[("/deg", self.deg)])?;
        let angle = self.deg.to_radians();
        let c = Change {
            view: self.view.as_ref(),
            viewport: self.viewport,
            pivot: self.pivot,
            gesture: self.gesture,
            end: self.end,
            remember: true,
            moves: Touches::TURN,
        };
        change(cx, &c, |camera, pivot, _, _| camera.roll(pivot, angle))
    }
}
register_command!(ViewRoll, "Roll");

impl Command for ViewPan {
    const NAME: &'static str = "view.pan";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Pivot;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "slide the model 100 pixels to the right",
                json!({ "dx": 100 }),
            )
            .after("primitive.sphere", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        finite(&[("/dx", self.dx), ("/dy", self.dy)])?;
        let (dx, dy) = (self.dx, self.dy);
        let size = super::size(self.viewport)?.map(|v| v as f32);
        // Align To Object pans a perspective view by the lens shift, part of the lens.
        let view = name(self.view.as_ref())?;
        let perspective = cx.views.camera_of(view, cx.doc.document())?.projection;
        let by_lens = cx.views.get(view).nav.align_to_object && perspective == Perspective;
        let c = Change {
            view: self.view.as_ref(),
            viewport: self.viewport,
            pivot: self.pivot,
            gesture: self.gesture,
            end: self.end,
            remember: true,
            moves: if by_lens { Touches::ZOOM } else { Touches::PAN },
        };
        change(cx, &c, |camera, pivot, _, _| {
            if by_lens {
                camera.pan_lens(dx, dy, size);
            } else {
                camera.pan(pivot, dx, dy, size);
            }
        })
    }
}
register_command!(ViewPan, "Pan");

impl Command for ViewDolly {
    const NAME: &'static str = "view.dolly";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Pivot;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("come twice as close", json!({ "factor": 0.5 }))
                .after("primitive.sphere", json!({})),
            Example::raw(
                "come closer to the surface under a pixel",
                json!({ "factor": 0.8, "toward": [600, 400] }),
            )
            .after("primitive.sphere", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        if !(self.factor > 0.0 && self.factor.is_finite()) {
            return Err(errors::args_invalid("/factor", "must be a positive number"));
        }
        let size = super::size(self.viewport)?.map(|v| v as f32);
        let (factor, toward) = (self.factor, self.toward);
        // Auto Adjust Distance, on anyway about the last edit (canvas.local-pivot).
        let seen = cx.views.get(name(self.view.as_ref())?);
        let scale = seen.nav.auto_distance || seen.pivot == PivotRule::LastEdit;
        let c = Change {
            view: self.view.as_ref(),
            viewport: self.viewport,
            pivot: self.pivot,
            gesture: self.gesture,
            end: self.end,
            remember: true,
            // Sliding keeps the distance and moves the target.
            moves: if scale { Touches::ZOOM } else { Touches::FRAME },
        };
        change(cx, &c, |camera, pivot, _, doc| {
            let about = toward.map_or(pivot, |[x, y]| {
                let (o, d) = camera.ray(x, y, size);
                let on_surface = super::super::frame::hit(doc, (o, d));
                // Over empty canvas: the point along the ray at the pivot's depth.
                let depth = camera.depth(pivot).max(camera.distance * 1e-3);
                on_surface.map_or_else(
                    || o + d * (depth / d.dot(camera.view().dir)),
                    gs_core::Vec3::from_array,
                )
            });
            if scale {
                camera.dolly(about, factor);
            } else {
                camera.advance(about, factor);
            }
        })
    }
}
register_command!(ViewDolly, "Dolly");
