//! `mesh.translate`, a temporary edit command that exists to exercise undo, effects and
//! lengths until the Gizmo arrives.

use gs_core::{ColumnSet, MeshStore, NodeId};
use gs_protocol::{CommandError, Kind, Length, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::resolve_length;
use crate::command::PartArg;
use crate::document::pinned;
use crate::math::dot;
use crate::transform::{Reads, edit_points};
use crate::{Command, Ctx, Example, errors, register_command};

/// Move a whole part a distance along a direction.
///
/// On a part with detail levels shown below its top, it moves the top's points when no edit
/// waits to be carried, so every level moves and switching level carries nothing.
///
/// Temporary: it will give way to the Gizmo's move.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshTranslate {
    #[doc = crate::command::part_doc!("The part to move.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Direction to move in. Need not be unit length, but must not be zero.
    pub direction: Vec3,
    /// How far to move: scene units, or a percentage such as "10%" of the part's bounding-box
    /// diagonal.
    pub distance: Length,
}

/// Where a part moved.
#[derive(Serialize, JsonSchema)]
pub struct Moved {
    /// The part that moved.
    pub part: String,
    /// The offset added to every vertex.
    pub offset: Vec3,
}

impl Command for MeshTranslate {
    const NAME: &'static str = "mesh.translate";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[
        "preserves_topology",
        {"local": {"part": "/part"}},
        {"volume_non_decreasing": {"tolerance": 2e-6}},
        {"mirror_equivariant": {"vectors": ["/direction"], "tolerance": 1e-6}}
    ]"#;
    type Output = Moved;

    fn examples() -> Vec<Example<Self>> {
        let up = Self {
            part: None,
            direction: Vec3([0.0, 1.0, 0.0]),
            distance: Length::Units(0.5),
        };
        let along_x = Self {
            part: Some(PartRef("cube".into())),
            direction: Vec3([1.0, 0.0, 0.0]),
            distance: Length::Percent(25.0),
        };
        vec![
            Example::new("move the only part up half a unit", up)
                .after("primitive.sphere", json!({})),
            Example::new("move a cube along x by a quarter of its size", along_x)
                .after("primitive.cube", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Moved, CommandError> {
        let part = pinned(self.part)?;
        // In f64, so neither huge nor tiny directions overflow or underflow on the way.
        let direction = self.direction.0.map(f64::from);
        let length = dot(direction, direction).sqrt();
        if !length.is_normal() {
            return Err(errors::args_invalid(
                "/direction",
                "direction must be a non-zero vector",
            ));
        }
        let distance = resolve_length(self.distance, cx.doc.diagonal(&part)?);
        let scale = f64::from(distance) / length;
        let offset = direction.map(|d| (d * scale) as f32);
        cx.noop_reason(format!(
            "moving by {offset:?} (distance {distance}) left every vertex where it was"
        ));
        // The store moves by the scene offset seen in the part's own space (decision 0029).
        let along = cx
            .doc
            .transform(&part)?
            .part_offset(gs_core::Vec3::from_array(offset));
        edit_points(cx, &part, Reads::Point, |s| shift(s, along))?;
        Ok(Moved {
            part,
            offset: Vec3(offset),
        })
    }
}
register_command!(MeshTranslate, "Move part");

/// Adds `along` to every point of `store`, a pure map (decision 0041, amended 2026-10-13), and
/// gives the nodes written: none for a move of zero, so it stays no step on any level.
fn shift(store: &mut MeshStore, along: gs_core::Vec3) -> Vec<NodeId> {
    if along == gs_core::Vec3::ZERO {
        return Vec::new();
    }
    let nodes: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
    store.write(&nodes, ColumnSet::POSITION).each(|node| {
        for p in node.positions() {
            *p += along;
        }
    });
    nodes
}
