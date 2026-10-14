//! `gizmo.describe` (decision 0027, section 3): a part's gizmo and its handles on a camera's
//! image, from the same function the window draws them with, and the open deformer's handles
//! (decision 0041, amended 2026-10-13 for `DF7c`, section 9).

use gs_core::camera::Camera;
use gs_core::config::NodeId;
use gs_core::gizmo::{
    Deformer, DeformerHandles, Style, deformer_handles, handles, pick, pick_deformer,
};
use gs_core::transform::Transform;
use gs_core::{Aabb, Vec3};
use gs_protocol::{CommandError, Cost, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{OpenDeformer, PoseInfo, pose_of};
use crate::command::PartArg;
use crate::document::pinned;
use crate::sculpt::{Axis, aim_camera};
use crate::transform::{OpArg, finite};
use crate::view::{NamedView, ViewName, ViewSpec, own_view};
use crate::{Command, Ctx, Example, register_command};

/// Describe a part's gizmo: where it is and its handles on a camera's image.
///
/// The handles are what the window draws: each one's outline in pixels from the image's
/// top-left corner, how strongly it is drawn (0 to 1), and whether a press can grab it. An
/// arrow or box pointing at the viewer fades out and cannot be grabbed, as a plane seen edge on
/// does. The gizmo keeps its size on screen at every zoom: `unit` says how long one gizmo unit
/// is in scene units there. While a deformer is open on the part, `deformer` has its box and
/// its handles the same way. Give `at` to ask which handle a press there would grab: a
/// deformer's before the gizmo's.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GizmoDescribe {
    #[doc = crate::command::part_doc!("The part whose gizmo it is.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The camera, as `view.snapshot` takes it: a standard view ("front"), a session view
    /// ("main"), an exact camera or an orbit. Left out, the caller's own view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<ViewSpec>,
    /// The camera's image size, `[width, height]` in pixels. Left out, `[1024, 1024]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[u32; 2]>,
    /// A point on the image, `[x, y]` in pixels: report the handle a press there would grab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
}

/// A part's gizmo.
#[derive(Serialize, JsonSchema)]
pub struct Described {
    /// The part whose gizmo it is.
    pub part: String,
    /// The gizmo.
    pub pose: PoseInfo,
    /// False while the gizmo was never placed: then it sits at the centre of the visible
    /// points with the scene's axes.
    pub placed: bool,
    /// One gizmo unit in scene units at the gizmo's centre; the handles' sizes are in these.
    pub unit: f32,
    /// One gizmo unit in pixels.
    pub unit_px: f32,
    /// The handles in the order a tie is broken in when picking. A handle with a point the
    /// camera cannot see (behind a perspective eye) is left out.
    pub handles: Vec<HandleInfo>,
    /// The deformer open on the part, if one is (`gizmo.deformer`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deformer: Option<DeformerInfo>,
    /// With `at`: the handle a press there would grab, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// The open deformer's box and handles on the image.
#[derive(Serialize, JsonSchema)]
pub struct DeformerInfo {
    /// One gizmo unit in scene units at the deformer's frame; the handles' sizes are in these.
    pub unit: f32,
    /// Its box's eight corners in the scene, then each repeat's where a Mirror or Parallel
    /// plane carries the deformer: corner `k` is at the box's high end on axis `a` when bit `a`
    /// of `k` is set. The handles are on the first box.
    pub boxes: Vec<[[f32; 3]; 8]>,
    /// Its handles, in the order a tie is broken in when picking.
    pub handles: Vec<HandleInfo>,
}

/// One handle on the image.
#[derive(Serialize, JsonSchema)]
pub struct HandleInfo {
    /// For example `move_x`, `move_view` (the centre square), `scale` (the grip),
    /// `rotate_view`; a deformer's `cone_x`, `ring_z`, `dot_1_2_0` (a cage point) or `point_0`
    /// (a curve point).
    pub name: String,
    /// `arrow`, `box`, `plane`, `centre`, `grip`, `ring` or `view_ring`; a deformer's `cone`,
    /// `ring` or `dot`.
    pub kind: &'static str,
    /// A deformer's: the argument it drags, a JSON pointer into its call's `op`
    /// (`/bend/degrees`, `/lattice/offsets`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drives: Option<&'static str>,
    /// The gizmo axis it acts along or about; for a plane, the axis across it; for a deformer's
    /// cone, the axis of the face it is on. Left out for the centre, the grip, the view ring
    /// and a dot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<Axis>,
    /// How strongly it is drawn, 0 to 1.
    pub fade: f32,
    /// Whether a press can grab it.
    pub pickable: bool,
    /// Its outline in pixels from the top-left corner: an arrow's two ends, a square's four
    /// corners, or a ring's points along its line.
    pub outline: Vec<[f32; 2]>,
}

impl Command for GizmoDescribe {
    const NAME: &'static str = "gizmo.describe";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    const PART: PartArg = PartArg::Existing;
    const CAMERA: Option<&'static str> = Some("camera");
    type Output = Described;

    fn examples() -> Vec<Example<Self>> {
        let front = Self {
            camera: Some(ViewSpec::Named(NamedView::Front)),
            at: Some([600.0, 512.0]),
            ..Self::default()
        };
        vec![
            Example::new(
                "the gizmo from the front, and what a press right of centre grabs",
                front,
            )
            .after("primitive.sphere", json!({ "subdivisions": 3 })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Described, CommandError> {
        let part = pinned(self.part.clone())?;
        let (pose, placed) = pose_of(cx, &part)?;
        let own = || ViewSpec::View(ViewName(own_view(cx.origin)));
        let spec = self.camera.clone().unwrap_or_else(own);
        let (camera, size) = aim_camera(&spec, self.viewport, cx.doc.document(), cx.views)?;
        let style = cx.window.with(crate::ui::WindowHost::gizmo_style);
        let shown = handles(&pose, &camera, size, &style);
        let open = (cx.gizmos.open_deformer()).filter(|d| d.part == part);
        let deformer = open.map(|d| deformer_of(cx, &d, (&camera, size, &style)));
        let deformer = deformer.transpose()?.flatten();
        let radius = style.pick_pt * style.px_per_pt;
        let at = (self.at).map(|at| finite("/at", at)).transpose()?;
        let at = at.and_then(|at| {
            let d = deformer.as_ref();
            let grabbed =
                d.and_then(|d| Some(d.handles[pick_deformer(&d.handles, at, radius)?].name()));
            grabbed.or_else(|| Some(pick(&shown.handles, at, radius)?.name().to_string()))
        });
        Ok(Described {
            part,
            pose: pose.into(),
            placed,
            unit: shown.unit,
            unit_px: shown.unit_px,
            handles: (shown.handles.into_iter())
                .map(|h| HandleInfo {
                    name: h.id.name().to_string(),
                    kind: h.id.kind(),
                    drives: None,
                    axis: h.id.axis().map(|a| AXES[a]),
                    fade: h.fade,
                    pickable: h.pickable,
                    outline: h.points,
                })
                .collect(),
            deformer: deformer.map(DeformerInfo::from),
            at,
        })
    }
}
register_command!(GizmoDescribe, "Describe gizmo");

const AXES: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

impl OpenDeformer {
    /// The transform its handles drag (decision 0041, amended 2026-10-13 for `DF7c`): its first
    /// call's. `None` for the smooth deformer, Scene tray's flatten, which has no handles, and
    /// a call that does not resolve.
    pub fn transform(&self) -> Option<Transform> {
        let call = self.transforms.first().filter(|_| self.smooth.is_none())?;
        if matches!(call.op, OpArg::Flatten(_)) {
            return None;
        }
        call.resolve(|| 1.0).ok()
    }
}

/// The open deformer `d`'s handles seen through `view`, on its part as it is now.
fn deformer_of(
    cx: &Ctx,
    d: &OpenDeformer,
    view: (&Camera, [f32; 2], &Style),
) -> Result<Option<DeformerHandles>, CommandError> {
    let Some(t) = d.transform() else {
        return Ok(None);
    };
    let found = cx.doc.get_part(&d.part)?;
    let store = found.store();
    let nodes = (0..store.node_count() as NodeId).map(|n| store.bounds(n));
    let bounds = nodes.fold(Aabb::EMPTY, Aabb::union);
    let placed = found.transform();
    let deformer = Deformer::of(&t, bounds, &placed);
    Ok(Some(deformer_handles(&t.frame, &deformer, view)))
}

impl From<DeformerHandles> for DeformerInfo {
    fn from(d: DeformerHandles) -> Self {
        DeformerInfo {
            unit: d.unit,
            boxes: (d.boxes.iter()).map(|b| b.map(Vec3::to_array)).collect(),
            handles: (d.handles.into_iter())
                .map(|h| HandleInfo {
                    name: h.name(),
                    kind: h.kind.name(),
                    drives: Some(h.drive.at()),
                    axis: h.axis.map(|a| AXES[a]),
                    fade: h.fade,
                    pickable: h.pickable,
                    outline: h.points,
                })
                .collect(),
        }
    }
}
