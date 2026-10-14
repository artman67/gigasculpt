//! `mesh.reorder` (decision 0021, L2): sorts a part's points along the Morton curve again and
//! cuts its nodes afresh, dropping dead slots, as one undo step. Live Detail keeps nodes local
//! stroke by stroke; this is the whole-part version, for after a long session of refining.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::{Order, Part, pinned, resort_spatial};
use crate::{Command, Ctx, Example, errors, register_command};

/// Sort a part's points in space again and cut its nodes afresh, so each node holds points
/// that lie together.
///
/// The surface does not change, but the points and faces get a new numbering: the store's own
/// order, also for a part made with a numbering of its own. UVs stay on their corners. One undo
/// step gives back the old numbering. A part with detail levels is refused.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshReorder {
    #[doc = crate::command::part_doc!("The part to reorder.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// What was reordered.
#[derive(Serialize, JsonSchema)]
pub struct Reordered {
    /// The part.
    pub part: String,
    /// Nodes before, empty ones included.
    pub nodes_before: usize,
    /// Nodes after.
    pub nodes: usize,
}

impl Command for MeshReorder {
    const NAME: &'static str = "mesh.reorder";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = Reordered;

    fn examples() -> Vec<Example<Self>> {
        let refining = json!({
            "brush": "Clay",
            "samples": [{ "position": [-0.3, 0.1, 0.95] }, { "position": [0.3, 0.2, 0.93] }],
            "size": "20%",
            "live_detail": { "mode": "constant", "size": 0.04 },
        });
        vec![
            Example::new(
                "reorder a sphere after a refining stroke",
                Self { part: None },
            )
            .after("primitive.sphere", json!({ "subdivisions": 6 }))
            .after("brush.stroke", refining),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Reordered, CommandError> {
        let part = pinned(self.part)?;
        cx.doc.single_level(&part, "a reorder")?;
        let old = cx.doc.get_part(&part)?;
        let nodes_before = old.store().node_count();
        let mut store = old.store().share();
        resort_spatial(&mut store, &old.order()).map_err(|e| errors::internal(&e.to_string()))?;
        let nodes = store.node_count();
        let new = Part::loaded(store, Order::Spatial, old.props());
        cx.noop_reason(format!("part '{part}' already was in sorted order"));
        // A swap, so the effect compares what `part.get` shows and an unchanged part is put back.
        cx.doc.replace_part(&part, new);
        Ok(Reordered {
            part,
            nodes_before,
            nodes,
        })
    }
}
register_command!(MeshReorder, "Reorder points");
