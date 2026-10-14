//! `gizmo.drag` (decision 0027, sections 7 and 8): the command checks an update, pins and
//! snaps it, and queues it for the drag in progress (`live`), which the engine applies after
//! the call. A Ctrl drag (`copy`, decision 0027, section 14) moves a live copy instead.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, json};

use super::live::{Event, Update};
use crate::transform::{self, FrameArg, TransformApply, finite, unit_quat};
use crate::{Command, Ctx, Example, errors, register_command};

/// Drag the gizmo: transform a part live, as one undo step.
///
/// For the window, while the pen drags a gizmo handle. `{"phase": "update", "transform": ...}`
/// carries the whole transform since the drag began, as `transform.apply` takes it; each
/// update starts again from the surface the drag began on. `{"phase": "end"}` keeps the result
/// as one undo step, logged as the `transform.apply` call that replays it; `{"phase":
/// "cancel"}` puts the part back exactly. While a drag is open, agents' edits wait; any edit,
/// undo or pen stroke ends it first, and the drag's later updates change nothing until its end
/// or cancel.
///
/// With `"copy": true` (a Ctrl drag) the updates move a copy of the part, which becomes the
/// active part; the end keeps it as the `part.duplicate` call that replays it, and a cancel
/// takes it away.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GizmoDrag {
    /// "update", "end" or "cancel".
    pub phase: Phase,
    /// With "update": the whole transform since the drag began, as `transform.apply` takes
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<TransformApply>,
    /// With "update": snap the turn to whole steps and the move to a spacing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<Snap>,
    /// With "update": move a copy of the part, not the part. The copy moves whole: the
    /// operation must be a move, a turn or an even scale, and the transform's mask, falloff
    /// and symmetry are not used.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub copy: bool,
}

/// Where a drag is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// The pen moved: here is the transform now.
    Update,
    /// The pen lifted: keep it.
    End,
    /// Esc: put the part back.
    Cancel,
}

/// How an update snaps.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Snap {
    /// A turn snaps to a whole number of these steps of a full turn: 72 (the default) is 5
    /// degrees, 36 is 10.
    #[serde(default = "steps")]
    #[schemars(range(min = 1, max = 3600))]
    pub steps: u32,
    /// A move snaps, along each of the frame's axes, to a multiple of this many scene units
    /// (the window gives 0.01, gs-core's `MOVE_SNAP`). Left out, a move does not snap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<f32>,
}

fn steps() -> u32 {
    72
}

/// What a drag call queued.
#[derive(Serialize, JsonSchema)]
pub struct Dragged {
    /// The part an update transforms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// An update as it will be applied and logged: pinned and snapped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<TransformApply>,
    /// A copy's update: the name the copy has.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy: Option<String>,
}

impl Command for GizmoDrag {
    const NAME: &'static str = "gizmo.drag";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const HUMAN_ONLY: bool = true;
    const LOGGED: bool = false;
    type Output = Dragged;

    fn examples() -> Vec<Example<Self>> {
        let sphere = json!({ "subdivisions": 3 });
        let turn = json!({
            "phase": "update",
            "transform": { "op": { "rotate": { "axis": "y", "degrees": 47.0 } } },
            "snap": {},
        });
        vec![
            Example::raw(
                "turn the sphere while the pen drags a ring, in 5 degree steps",
                turn,
            )
            .after("primitive.sphere", sphere.clone()),
            Example::raw(
                "lift the pen: keep it as one undo step",
                json!({ "phase": "end" }),
            )
            .after("primitive.sphere", sphere.clone()),
            Example::raw(
                "Ctrl drag an arrow: move a copy of the sphere 2 units along X",
                json!({
                    "phase": "update",
                    "transform": { "op": { "move": [2.0, 0.0, 0.0] } },
                    "copy": true,
                }),
            )
            .after("primitive.sphere", sphere.clone()),
            Example::raw(
                "a copy moves whole: a stretch is refused",
                json!({
                    "phase": "update",
                    "transform": { "op": { "scale": [2.0, 1.0, 1.0] } },
                    "copy": true,
                }),
            )
            .after("primitive.sphere", sphere.clone())
            .fails_with(ErrorCode::ArgsInvalid),
            Example::raw(
                "a scale below 0 is refused: it turns the sphere inside out",
                json!({ "phase": "update", "transform": { "op": { "scale": [-1.0, 1.0, 1.0] } } }),
            )
            .after("primitive.sphere", sphere)
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Dragged, CommandError> {
        if self.copy && self.phase != Phase::Update {
            let message = "only an update moves a copy";
            return Err(errors::args_invalid("/copy", message));
        }
        let event = match (self.phase, self.transform, self.snap) {
            (Phase::Update, Some(args), snap) => {
                if let Some(job) = cx.control.held {
                    return Err(crate::job::busy(job));
                }
                // An edit committed this drag and cut it: the rest of it changes nothing, as a
                // stroke's samples after an edit find no stroke.
                if cx.gizmos.cut {
                    return Ok(Dragged {
                        part: None,
                        transform: None,
                        copy: None,
                    });
                }
                let update = update(cx, args, snap, self.copy)?;
                cx.control.gesture = true;
                let out = Dragged {
                    part: update.args.part.clone().map(|p| p.0),
                    transform: Some(update.args.clone()),
                    copy: update.copy.clone(),
                };
                cx.gizmos.queued.push(Event::Update(Box::new(update)));
                return Ok(out);
            }
            (Phase::Update, None, _) => {
                let message = "an update needs the transform";
                return Err(errors::args_invalid("/transform", message));
            }
            (Phase::End, None, None) => Event::End,
            (Phase::Cancel, None, None) => Event::Cancel,
            (_, Some(_), _) => {
                let message = "only an update takes a transform";
                return Err(errors::args_invalid("/transform", message));
            }
            (_, None, Some(_)) => {
                let message = "only an update snaps";
                return Err(errors::args_invalid("/snap", message));
            }
        };
        cx.control.gesture = false;
        cx.gizmos.cut = false;
        cx.gizmos.queued.push(event);
        Ok(Dragged {
            part: None,
            transform: None,
            copy: None,
        })
    }
}
register_command!(GizmoDrag, "Drag gizmo");

/// Pins `args` (the part, and the frame the drag on that part began with, else the gizmo's),
/// snaps it and resolves it against the surface the drag on that part began on. For a `copy`, also pins the copy's name (the open copy drag's, else
/// `part.duplicate`'s default) and refuses an operation that does not move the copy whole.
fn update(
    cx: &Ctx,
    mut args: TransformApply,
    snap: Option<Snap>,
    copy: bool,
) -> Result<Update, CommandError> {
    let open = (cx.gizmos.drag.as_ref()).and_then(|d| Some((d, d.copy.as_ref()?)));
    if copy
        && args.part.is_none()
        && let Some((_, copied)) = open
    {
        // The live copy is the active part now; the drag still copies the part it began on.
        args.part = Some(PartRef(copied.source.clone()));
    }
    let mut map = Map::new();
    if let Some(part) = &args.part {
        map.insert("part".into(), part.0.clone().into());
    }
    crate::pinning::pin_existing(&cx.doc, &mut map)?;
    let part = map["part"].as_str().unwrap_or_default().to_string();
    args.part = Some(PartRef(part.clone()));
    // The gizmo where the drag on this part began: a gizmo not placed sits at the middle of
    // what shows, which the drag itself moves, so later updates keep the first one's frame.
    let began = (cx.gizmos.drag.as_ref())
        .filter(|d| match &d.copy {
            Some(c) => copy && c.source == part,
            None => !copy && d.part == part,
        })
        .map(|d| d.args.frame);
    match began {
        Some(frame) if matches!(args.frame, FrameArg::Gizmo(_)) => args.frame = frame,
        _ => args.pin_frame(cx, &part)?,
    }
    if let Some(snap) = snap {
        snapped(&mut args, snap)?;
    }
    let diagonal = match &cx.gizmos.drag {
        Some(drag) if drag.part == part => drag.diagonal,
        _ => cx.doc.get_part(&part)?.world_diagonal(),
    };
    let t = args.resolve(|| diagonal)?;
    let copy = match open {
        _ if !copy => None,
        Some((drag, copied)) if copied.source == part => Some(drag.part.clone()),
        _ => {
            let mut name = None;
            crate::part::pin_copy_name(cx, args.part.as_ref(), &mut name)?;
            name
        }
    };
    if copy.is_some() && transform::placed(cx.doc.get_part(&part)?.props().transform, &t).is_none()
    {
        let message = "a copy moves, turns or scales evenly as a whole: move, rotate, or scale by \
                       three equal factors above zero";
        return Err(errors::args_invalid("/transform/op", message));
    }
    Ok(Update {
        origin: cx.origin.clone(),
        args,
        t,
        diagonal,
        copy,
    })
}

/// Snaps a turn to whole steps of a full turn, and a move's parts along the frame's axes to
/// multiples of the spacing (decision 0027, section 8).
fn snapped(args: &mut TransformApply, snap: Snap) -> Result<(), CommandError> {
    use transform::OpArg;
    let frame = args.exact()?;
    match &mut args.op {
        OpArg::Rotate(r) => {
            if !(1..=3600).contains(&snap.steps) {
                let message = "steps must be from 1 to 3600";
                return Err(errors::args_invalid("/snap/steps", message));
            }
            r.degrees = gs_core::gizmo::snap_turn(r.degrees, snap.steps);
        }
        OpArg::Move(d) => {
            let Some(spacing) = snap.spacing else {
                return Ok(());
            };
            if !(spacing > 0.0 && spacing.is_finite()) {
                let message = "spacing must be a distance above 0";
                return Err(errors::args_invalid("/snap/spacing", message));
            }
            let q = unit_quat(frame.axes).ok_or_else(|| {
                errors::args_invalid("/transform/frame/axes", "axes must not be zero")
            })?;
            let v = gs_core::Vec3::from_array(finite("/transform/op/move", d.0)?);
            let axes = gs_core::Quat::from_array(q);
            d.0 = gs_core::gizmo::snap_move(v, axes, spacing).to_array();
        }
        _ => {}
    }
    Ok(())
}
