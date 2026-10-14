//! `view.frame`, `view.align` and `view.level`: fitting what matters in the picture, the
//! standard views, and taking a lean away.

use gs_core::Aabb;
use gs_core::camera::{Camera, OrbitMode};
use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::super::frame::{box_corners, mask_box, part_box, scene_box};
use super::super::reference::{fit_sphere, sphere};
use super::super::spec::{NamedView, ViewName};
use super::super::views::locked;
use super::{Change, Moved, Touches, change, may_change, name};
use crate::command::ViewArg;
use crate::errors::{self, error};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Why a view cannot be centred on its last edit point.
pub(crate) const NO_LAST_EDIT: &str =
    "no stroke has touched the surface on this view's behalf since it was framed";

/// What `view.frame` fits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FrameTarget {
    /// Every part.
    #[default]
    All,
    /// The part named by `part`.
    Part,
    /// The points the mask covers at all.
    Mask,
    /// The points the mask leaves at least partly free.
    Unmasked,
    /// Centre on where the last stroke on the view's behalf first touched, at the same distance.
    LastEdit,
    /// Centre on `point`, at the same distance.
    Point,
}

/// Fit what matters in a view, keeping the turn: every part, one part, the masked or the
/// unmasked points with a 5% border, or centre on the last edit point or a given point. Only
/// the faces that show count, and only the parts that show unless one is named. A Reference
/// View's every part is fitted by the sphere around them, so linked turns keep it in the
/// picture.
///
/// Afterwards the view turns about what it framed until the next stroke. One step of the
/// view's camera history; never part of document undo.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewFrame {
    /// The view to frame: "main", "agent:<name>" or "ref:<n>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels; framing fits its
    /// shape. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// What to fit. Left out, it is a press of Frame: every part, except that right after a
    /// framing of every part, with the view not moved since and two parts or more showing, it
    /// is the active part (another press goes back to every part). It is pinned to which.
    #[serde(default)]
    pub target: FrameTarget,
    /// With target `part`: which part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// With target `point`: the point, in scene units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<Vec3>,
}

/// Pins a press of Frame on view `name` (`target` left out) to what it frames: the active part
/// (decision 0033) when the view still shows the camera its last framing of every part gave and
/// two parts or more show in the session's scene, else every part.
pub(crate) fn press(cx: &Ctx, name: &str, map: &mut Map<String, Value>) {
    let view = cx.views.get(name);
    let doc = cx.doc.document();
    let again = view.framed_all.is_some() && view.camera == view.framed_all;
    let several = cx.views.secondary(name).is_none() && doc.shown().nth(1).is_some();
    match doc.active().filter(|_| again && several) {
        Some(part) => {
            map.insert("target".into(), json!(FrameTarget::Part));
            map.insert("part".into(), json!(part));
        }
        None => {
            map.insert("target".into(), json!(FrameTarget::All));
        }
    }
}

impl ViewFrame {
    /// The box to fit in the scene the view shows, or the point to centre on.
    fn aim(&self, cx: &Ctx, view: &str) -> Result<Result<Aabb, gs_core::Vec3>, CommandError> {
        let doc = cx.views.scene(view, cx.doc.document());
        let fit = |b: Aabb, what: &str| {
            if b == Aabb::EMPTY {
                Err(error(
                    ErrorCode::Precondition,
                    format!("there is nothing to frame: {what}"),
                ))
            } else {
                Ok(Ok(b))
            }
        };
        let centre = |p: Option<[f32; 3]>, what: &str| {
            let p = p.ok_or_else(|| error(ErrorCode::Precondition, what.to_string()))?;
            Ok(Err(gs_core::Vec3::from_array(p)))
        };
        match self.target {
            FrameTarget::All => fit(scene_box(doc), "no part shows"),
            FrameTarget::Part => {
                let PartRef(part) = (self.part.clone())
                    .ok_or_else(|| errors::args_invalid("/part", "name the part to frame"))?;
                let Some(name) = doc.resolve(&part) else {
                    return Err(errors::not_found("part", &part, doc.names()));
                };
                fit(part_box(doc, name), "the part is empty")
            }
            FrameTarget::Mask => fit(mask_box(doc, |m| m > 0.0), "no point is masked"),
            FrameTarget::Unmasked => fit(mask_box(doc, |m| m < 1.0), "every point is masked"),
            FrameTarget::LastEdit => centre(cx.views.get(view).last_edit, NO_LAST_EDIT),
            FrameTarget::Point => centre(
                self.point.map(|p| p.0),
                "target point needs `point`, for example [0, 1, 0]",
            ),
        }
    }
}

impl Command for ViewFrame {
    const NAME: &'static str = "view.frame";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Frame;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("fit every part", json!({})).after("primitive.sphere", json!({})),
            Example::raw("fit one part", json!({ "target": "part", "part": "cube" }))
                .after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({ "center": [3, 0, 0] })),
            Example::raw("fit the masked points", json!({ "target": "mask" }))
                .after("primitive.sphere", json!({}))
                .fails_with(ErrorCode::Precondition),
            Example::raw(
                "centre on a point",
                json!({ "target": "point", "point": [0, 1, 0] }),
            )
            .after("primitive.sphere", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        let view = name(self.view.as_ref())?.to_string();
        let aim = self.aim(cx, &view)?;
        // A whole Reference View is framed by the sphere around it, which stays in its picture
        // however main turns it (decision 0024, section 5).
        let whole = self.target == FrameTarget::All && cx.views.secondary(&view).is_some();
        let around = whole
            .then(|| sphere(cx.views.scene(&view, cx.doc.document())))
            .flatten();
        let [w, h] = super::size(self.viewport)?;
        let aspect = w as f32 / h as f32;
        let c = Change {
            view: self.view.as_ref(),
            viewport: self.viewport,
            pivot: None,
            gesture: None,
            end: false,
            remember: true,
            moves: Touches::FRAME,
        };
        let moved = change(cx, &c, |camera, _, _, _| match (aim, around) {
            (_, Some(around)) => fit_sphere(camera, around, aspect),
            (Ok(b), None) => camera.fit(&box_corners(b), aspect),
            (Err(p), None) => camera.target = p.to_array(),
        })?;
        let all = self.target == FrameTarget::All;
        cx.views.settle(&view, |v| {
            v.last_edit = None;
            v.framed_all = if all { v.camera } else { None };
        });
        Ok(moved)
    }
}
register_command!(ViewFrame, "Frame");

/// Turn a view to a standard view about its pivot, keeping its distance, projection and lens,
/// with a level horizon (up is +Y from the sides; from the top the model's back is at the
/// top of the picture).
///
/// With `toggle`, a view already there turns to the opposite side: front then back, left then
/// right, top then bottom, as repeated clicks on the corner figure's axis ends do. Without it a
/// view already there stays. One step of the view's camera history.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewAlign {
    /// The view to turn: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// The standard view: front, back, left, right, top, bottom or iso.
    pub to: NamedView,
    /// Already there: go to the opposite side instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub toggle: bool,
    /// The point to turn about. Left out: the view's pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<Vec3>,
    /// The pixel the move starts at, for the `under-pointer` pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
}

/// The standard view on the other side of `v`; iso has none.
fn opposite(v: NamedView) -> NamedView {
    match v {
        NamedView::Front => NamedView::Back,
        NamedView::Back => NamedView::Front,
        NamedView::Left => NamedView::Right,
        NamedView::Right => NamedView::Left,
        NamedView::Top => NamedView::Bottom,
        NamedView::Bottom => NamedView::Top,
        NamedView::Iso => NamedView::Iso,
    }
}

/// `camera` looks along `v` already, up to a rounding.
fn looks_along(camera: &Camera, v: NamedView) -> bool {
    let a = camera.orientation.to_array();
    let b = v.preset().orientation().to_array();
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    dot.abs() > 1.0 - 1e-6
}

impl Command for ViewAlign {
    const NAME: &'static str = "view.align";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Pivot;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("the front view", json!({ "to": "front" }))
                .after("primitive.sphere", json!({})),
            Example::raw(
                "front, or back if already front",
                json!({ "to": "front", "toggle": true }),
            )
            .after("primitive.sphere", json!({})),
            Example::raw(
                "the iso view, turning about a point",
                json!({ "to": "iso", "pivot": [0, 1, 0] }),
            )
            .after("primitive.cube", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        let view = name(self.view.as_ref())?;
        let now = cx.views.camera_of(view, cx.doc.document())?;
        let there = looks_along(&now, self.to);
        if there && !self.toggle {
            return Ok(Moved {
                view: view.to_string(),
                camera: now.into(),
            });
        }
        let to = if there { opposite(self.to) } else { self.to };
        let c = Change {
            view: self.view.as_ref(),
            viewport: self.viewport,
            pivot: self.pivot,
            gesture: None,
            end: false,
            remember: true,
            moves: Touches::TURN,
        };
        let turn = to.preset().orientation();
        change(cx, &c, |camera, pivot, _, _| camera.turn_to(turn, pivot))
    }
}
register_command!(ViewAlign, "Align view");

/// Take the lean out of a view: turn it about its direction through the pivot until the
/// screen's horizontal axis is level and the picture upright, keeping where it looks (its
/// heading and height). Level means level with the axis of the view's orbit mode, the scene's
/// +Y in free mode. A view already level, or looking straight along that axis, stays, with no
/// history step; otherwise one step of the view's camera history.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewLevel {
    /// The view to level: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// The point to turn about. Left out: the view's pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<Vec3>,
    /// The pixel the move starts at, for the `under-pointer` pivot rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
}

impl Command for ViewLevel {
    const NAME: &'static str = "view.level";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Pivot;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("level a rolled view", json!({}))
                .after("primitive.sphere", json!({}))
                .after("view.roll", json!({ "deg": 30 })),
            Example::raw("a level view stays", json!({})).after("primitive.cube", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        let view = name(self.view.as_ref())?;
        // A locked view refuses it even when level, as it refuses every move.
        if cx.views.get(view).locked {
            return Err(locked(view));
        }
        let c = Change {
            view: self.view.as_ref(),
            viewport: self.viewport,
            pivot: self.pivot,
            gesture: None,
            end: false,
            remember: true,
            moves: Touches::TURN,
        };
        match level(cx, &c)? {
            Some(moved) => Ok(moved),
            None => Ok(Moved {
                view: view.to_string(),
                camera: cx.views.camera_of(view, cx.doc.document())?.into(),
            }),
        }
    }
}
register_command!(ViewLevel, "Level view");

/// Levels the view `c` names (see [`ViewLevel`]), or `None` when it is level already.
pub(super) fn level(cx: &mut Ctx, c: &Change) -> Result<Option<Moved>, CommandError> {
    let view = name(c.view)?;
    let now = cx.views.camera_of(view, cx.doc.document())?;
    let up = OrbitMode::from(cx.views.get(view).orbit).up();
    let Some(turn) = now.leveled(up) else {
        return Ok(None);
    };
    change(cx, c, |camera, pivot, _, _| camera.turn_to(turn, pivot)).map(Some)
}
