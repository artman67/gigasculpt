//! `gizmo.mesh_to_axis` and `gizmo.reset_orientation`, the locked forms (decision 0029, section
//! 10): the whole part moves or turns so its gizmo lands on the scene origin or the scene axes,
//! and the gizmo goes with it. Edits, one undo step each, that change only the part's transform;
//! the gizmo's new pose is session state, as `gizmo.place`'s is. The unlocked forms, which move
//! only the gizmo, are `gizmo.place` with `world_origin` and `gizmo.reset`.

use gs_core::Vec3;
use gs_core::gizmo::Pose;
use gs_core::transform::Similarity;
use gs_protocol::{CommandError, Cost, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::place::Placed;
use super::{pose_of, set_pose};
use crate::command::PartArg;
use crate::document::{Props, pinned};
use crate::{Command, Ctx, Example, errors, register_command};

/// Move a part so its gizmo's centre lands on the scene origin (Mesh to Axis, with the gizmo
/// locked). The whole part moves, hidden and masked points too, and keeps its offset from the
/// gizmo; the gizmo keeps its axes. One undo step.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GizmoMeshToAxis {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

impl Command for GizmoMeshToAxis {
    const NAME: &'static str = "gizmo.mesh_to_axis";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Instant;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[
        "preserves_topology",
        {"local": {"part": "/part"}}
    ]"#;
    type Output = Placed;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "bring a sphere's gizmo, and the sphere, to the origin",
                json!({}),
            )
            .after("primitive.sphere", json!({ "subdivisions": 3 }))
            .after(
                "gizmo.place",
                json!({ "origin": [0, 1, 0], "normal": [0, 1, 0] }),
            ),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Placed, CommandError> {
        let part = pinned(self.part)?;
        let (pose, _) = pose_of(cx, &part)?;
        place(cx, &part, Similarity::moving(-pose.origin))?;
        let pose = Pose {
            origin: Vec3::ZERO,
            ..pose
        };
        set_pose(cx, &part, pose)?;
        Ok(Placed {
            part,
            pose: pose.into(),
        })
    }
}
register_command!(GizmoMeshToAxis, "Mesh to axis");

/// Turn a part about its gizmo's centre so the gizmo's axes become the scene's (Reset
/// Orientation, with the gizmo locked): a part turned with the gizmo ends square to the scene.
/// The whole part turns, hidden and masked points too; the gizmo keeps its centre. One undo
/// step.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GizmoResetOrientation {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

impl Command for GizmoResetOrientation {
    const NAME: &'static str = "gizmo.reset_orientation";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Instant;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"["preserves_topology", {"local": {"part": "/part"}}]"#;
    // Changes something only on a part whose gizmo is turned, and only `gizmo.place`, a View
    // command random plans never call, turns it: its claims are checked on plans that start by
    // tilting each part's gizmo.
    const SETUP: &'static str = r#"[{"command": "gizmo.place", "args": {
        "origin": [0, 0, 0], "normal": [1, 1, 0]
    }}]"#;
    type Output = Placed;

    fn examples() -> Vec<Example<Self>> {
        let tilted = json!({ "origin": [0, 0, 0], "normal": [1, 1, 0] });
        vec![
            Example::raw("square a cube to its tilted gizmo's turn", json!({}))
                .after("primitive.cube", json!({}))
                .after("gizmo.place", tilted),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Placed, CommandError> {
        let part = pinned(self.part)?;
        let (pose, _) = pose_of(cx, &part)?;
        place(
            cx,
            &part,
            Similarity::turning(pose.origin, pose.axes.inverse()),
        )?;
        let pose = Pose {
            axes: gs_core::Quat::IDENTITY,
            ..pose
        };
        set_pose(cx, &part, pose)?;
        Ok(Placed {
            part,
            pose: pose.into(),
        })
    }
}
register_command!(GizmoResetOrientation, "Reset orientation");

/// Moves part `part` by `op`, after its transform: a move or turn of the whole part is one its
/// transform always holds (decision 0029, section 6).
fn place(cx: &mut Ctx, part: &str, op: Similarity) -> Result<(), CommandError> {
    let props = cx.doc.get_part(part)?.props();
    let transform = props.transform.then(&op);
    if !transform.is_valid() {
        let why = "the part would end too far from the origin to keep its place";
        return Err(errors::unavailable(crate::Unavailable::new(why)));
    }
    if transform == props.transform {
        cx.noop_reason("the gizmo already sits there, so the part stays where it is");
    }
    cx.doc.set_props(part, Props { transform, ..props })
}
