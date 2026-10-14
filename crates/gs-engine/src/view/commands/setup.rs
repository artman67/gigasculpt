//! `view.lens`, `view.lock`, `view.settings` and `view.touch`: how a view projects, whether it
//! may move, how navigation moves it, and the point a right drag over the model turns it about.

use gs_core::camera::{Lens, Projection};
use gs_protocol::{CommandError, Cost, Kind, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::spec::{FitArg, LensArg, ViewName};
use super::super::views::{NavSettings, OrbitArg, PivotRule, ViewInfo};
use super::{Change, Moved, Touches, change, may_change, name};
use crate::command::ViewArg;
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// Perspective, orthographic, or the other one of the two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProjectionChange {
    Perspective,
    Orthographic,
    /// Whichever the view does not use now.
    Toggle,
}

/// Change how a view projects: perspective or orthographic (orthographic shows the height a
/// perspective camera sees at the target, so switching keeps the framing), the focal length or
/// field of view, which side of the image the lens's 36 by 24 mm frame spans, and the lens
/// shift. Leave out what stays. One step of the view's camera history.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewLens {
    /// The view: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// `perspective`, `orthographic`, or `toggle` for the other one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<ProjectionChange>,
    /// Focal length in millimetres: 29 is wide, 85 and longer flatten the perspective.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub focal_mm: Option<f32>,
    /// Field of view in degrees across the side the frame spans, instead of a focal length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.1, max = 179.0))]
    pub fov_deg: Option<f32>,
    /// Which side of the image the frame spans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<FitArg>,
    /// Where the view axis meets the image, from its centre, in image heights: `[right, up]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shift: Option<[f32; 2]>,
}

impl ViewLens {
    /// The lens after the change, from the lens `now`.
    fn lens(&self, now: Lens) -> Result<Lens, CommandError> {
        let mut lens = LensArg::from(now);
        lens.fit = self.fit.unwrap_or(lens.fit);
        lens.shift = self.shift.unwrap_or(lens.shift);
        lens.focal_mm = self.focal_mm.unwrap_or(lens.focal_mm);
        let mut lens = lens.lens("")?;
        if let Some(fov) = self.fov_deg {
            if self.focal_mm.is_some() {
                let message = "give focal_mm or fov_deg, not both";
                return Err(errors::args_invalid("/fov_deg", message));
            }
            if !(0.1..=179.0).contains(&fov) {
                return Err(errors::args_invalid(
                    "/fov_deg",
                    "must be 0.1 to 179 degrees",
                ));
            }
            lens.focal_mm = Lens::from_fov(fov.to_radians(), lens.fit).focal_mm;
        }
        Ok(lens)
    }
}

impl Command for ViewLens {
    const NAME: &'static str = "view.lens";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::View;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "switch between perspective and orthographic",
                json!({ "projection": "toggle" }),
            )
            .after("primitive.sphere", json!({})),
            Example::raw("a long lens for a portrait", json!({ "focal_mm": 85 }))
                .after("primitive.sphere", json!({})),
            Example::raw("35 degrees from top to bottom", json!({ "fov_deg": 35 })),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        let view = name(self.view.as_ref())?;
        let now = cx.views.camera_of(view, cx.doc.document())?;
        let lens = self.lens(now.lens)?;
        let projection = match (self.projection, now.projection) {
            (None, p) => p,
            (Some(ProjectionChange::Perspective), _)
            | (Some(ProjectionChange::Toggle), Projection::Orthographic) => Projection::Perspective,
            (Some(ProjectionChange::Orthographic), _)
            | (Some(ProjectionChange::Toggle), Projection::Perspective) => Projection::Orthographic,
        };
        let c = Change {
            view: self.view.as_ref(),
            viewport: self.viewport,
            pivot: None,
            gesture: None,
            end: false,
            remember: true,
            moves: Touches::ZOOM,
        };
        change(cx, &c, |camera, _, _, _| {
            camera.lens = lens;
            camera.projection = projection;
        })
    }
}
register_command!(ViewLens, "Set lens");

/// Lock a view so no command and no gesture can move its camera: every
/// camera change refuses with a reason, and in the window a pen press on empty canvas sculpts
/// instead of turning the view. Leave `locked` out to switch.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewLock {
    /// The view: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// True to lock, false to unlock. Left out: the other way from now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
}

impl Command for ViewLock {
    const NAME: &'static str = "view.lock";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Name;
    type Output = ViewInfo;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("lock your view", json!({ "locked": true })),
            Example::raw("switch the lock", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<ViewInfo, CommandError> {
        let view = name(self.view.as_ref())?;
        cx.views.close(view);
        let locked = self.locked;
        cx.views
            .settle(view, |v| v.locked = locked.unwrap_or(!v.locked));
        Ok(cx.views.info(view, cx.doc.document()))
    }
}
register_command!(ViewLock, "Lock view");

/// Change how navigation turns a view: what a horizontal drag turns about (`x`, `y`, `z`: a
/// scene axis, `y`, the default, a turntable; `free`: the screen's vertical axis, a free
/// turn), and where it turns and zooms about (`target`, the default: the point the camera
/// looks at, the middle of the picture; `last-edit`: where the last stroke first touched;
/// `part`: the centre of every part; `under-pointer`: the surface under the press); and how
/// its moves behave: the recenter rate, the snap step, whether a zoom changes the distance,
/// whether a pan moves by the lens and whether a right drag moves the pivot. Leave out what stays.
///
/// Changing to a scene axis levels the view once, about the point it looks at, as `view.level`
/// does (one step of its camera history), unless the view is locked or its link holds its turn.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewSettings {
    /// The view: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// What a horizontal drag turns about: `free`, `x`, `y` or `z`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orbit: Option<OrbitArg>,
    /// Where the view turns and zooms about: `target`, `last-edit`, `part` or `under-pointer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot_rule: Option<PivotRule>,
    /// The share of the way to the middle of the picture an orbit slides the pivot per 90
    /// degrees of turn, 0 to 1 (0 keeps the pivot where it is on the screen).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.0, max = 1.0))]
    pub recenter_rate: Option<f32>,
    /// The step a snapping orbit lands on, in whole degrees dividing 90 (90: the axis views).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 90))]
    pub snap_deg: Option<u32>,
    /// Auto Adjust Distance: true, a zoom changes the camera's distance; false, in
    /// perspective, it slides the camera keeping its distance (on anyway with `last-edit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_distance: Option<bool>,
    /// Align To Object: true, a pan in perspective slides the picture by the lens shift; false,
    /// it slides the camera.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align_to_object: Option<bool>,
    /// Right-drag moves the pivot: whether `view.touch` moves the last-edit point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_drag_pivot: Option<bool>,
}

impl ViewSettings {
    /// The view's move settings after the change, from `now`.
    fn nav(&self, now: NavSettings) -> Result<NavSettings, CommandError> {
        let rate = self.recenter_rate.unwrap_or(now.recenter_rate);
        if !(0.0..=1.0).contains(&rate) {
            return Err(errors::args_invalid("/recenter_rate", "must be 0 to 1"));
        }
        let snap = self.snap_deg.unwrap_or(now.snap_deg);
        if !(1..=90).contains(&snap) || 90 % snap != 0 {
            let message = "must be whole degrees dividing 90, for example 15, 45 or 90";
            return Err(errors::args_invalid("/snap_deg", message));
        }
        Ok(NavSettings {
            recenter_rate: rate,
            snap_deg: snap,
            auto_distance: self.auto_distance.unwrap_or(now.auto_distance),
            align_to_object: self.align_to_object.unwrap_or(now.align_to_object),
            right_drag_pivot: self.right_drag_pivot.unwrap_or(now.right_drag_pivot),
        })
    }
}

impl Command for ViewSettings {
    const NAME: &'static str = "view.settings";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::Name;
    type Output = ViewInfo;

    fn examples() -> Vec<Example<Self>> {
        let pairs = [
            ("turn about the screen's axes", json!({ "orbit": "free" })),
            (
                "turn about the last stroke",
                json!({ "pivot_rule": "last-edit" }),
            ),
            (
                "turn about the surface under the pen",
                json!({ "pivot_rule": "under-pointer" }),
            ),
            (
                "snap every 15 degrees and drift the pivot to the middle",
                json!({ "snap_deg": 15, "recenter_rate": 0.5 }),
            ),
            (
                "pan by the lens and zoom without changing the perspective",
                json!({ "align_to_object": true, "auto_distance": false }),
            ),
            (
                "a right press over the model leaves the pivot where it is",
                json!({ "right_drag_pivot": false }),
            ),
        ];
        pairs
            .into_iter()
            .map(|(summary, args)| Example::raw(summary, args))
            .collect()
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<ViewInfo, CommandError> {
        let view = name(self.view.as_ref())?;
        let (orbit, pivot) = (self.orbit, self.pivot_rule);
        let was = cx.views.get(view).orbit;
        let nav = self.nav(cx.views.get(view).nav)?;
        cx.views.settle(view, |v| {
            v.orbit = orbit.unwrap_or(v.orbit);
            v.pivot = pivot.unwrap_or(v.pivot);
            v.nav = nav;
        });
        let axis = orbit.is_some_and(|o| o != was && o != OrbitArg::Free);
        if axis && cx.views.may_turn(view) {
            let target = cx.views.camera_of(view, cx.doc.document())?.target;
            let c = Change {
                view: self.view.as_ref(),
                viewport: Some(cx.views.get(view).viewport),
                pivot: Some(Vec3(target)),
                gesture: None,
                end: false,
                remember: true,
                moves: Touches::TURN,
            };
            super::frame::level(cx, &c)?;
        }
        Ok(cx.views.info(view, cx.doc.document()))
    }
}
register_command!(ViewSettings, "View settings");

/// Count a press at a pixel as a touch of the surface (canvas.local-pivot): the view's last-edit
/// point, which the `last-edit` pivot rule turns and zooms about and Frame last edit frames,
/// moves to the surface under the pixel, as a stroke's first touch moves it. The window sends it
/// when a right drag starts a move over the model. Over empty canvas, or with the view's
/// `right_drag_pivot` setting off, it changes nothing. It never moves the camera.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewTouch {
    /// The view: "main", or "agent:<name>". Left out: yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// The image size the view is seen at, `[width, height]` in pixels. Left out: the view's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// The pixel pressed, `[x, y]` from the top-left corner.
    pub at: [f32; 2],
}

impl Command for ViewTouch {
    const NAME: &'static str = "view.touch";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const VIEW: ViewArg = ViewArg::View;
    type Output = ViewInfo;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "a right press over the middle of the model",
                json!({ "at": [512, 512] }),
            )
            .after("primitive.sphere", json!({})),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        may_change(cx, self.view.as_ref())
    }

    fn run(self, cx: &mut Ctx) -> Result<ViewInfo, CommandError> {
        let view = name(self.view.as_ref())?;
        let size = super::size(self.viewport)?.map(|v| v as f32);
        if cx.views.get(view).nav.right_drag_pivot {
            let doc = cx.doc.document();
            let camera = cx.views.camera_of(view, doc)?;
            let [x, y] = self.at;
            let scene = cx.views.scene(view, doc);
            if let Some(point) = super::super::frame::hit(scene, camera.ray(x, y, size)) {
                cx.views.edited(view, point);
            }
        }
        Ok(cx.views.info(view, cx.doc.document()))
    }
}
register_command!(ViewTouch, "Touch the surface");
