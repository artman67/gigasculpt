//! `part.list`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::document::{Isolate, Operation};
use crate::{Command, Ctx, Example, register_command};

/// List the parts in order, each with its stable id, place, name, rank, visibility and Live
/// Boolean marks, and which one is active.
///
/// Any part argument takes `#id` (the id never changes, so keep it), `@place` (from 0, in this
/// order), `@active` or the name. Left out, it means the active part (decision 0033). There
/// are no folders yet, so no row names one (decision 0028, section 10).
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartList {}

/// One part of the list.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about a part, each one field of the row agents read"
)]
#[derive(Serialize, JsonSchema)]
pub struct PartRow {
    /// Stable id.
    pub id: u64,
    /// Place in the list, from 0.
    pub index: u64,
    pub name: String,
    pub vertices: u64,
    pub faces: u64,
    /// Where it sorts: the list is in rank order, then id order. A rank is not a place; it
    /// changes when parts move (decision 0028).
    pub rank: u64,
    /// Its eye is on.
    pub visible: bool,
    /// What Isolate made of it.
    pub isolate: Isolate,
    /// It shows: it is the active part, or Isolate chose it, or Isolate did not set it aside
    /// and its eye is on. A part that does not show is not drawn, picked or framed.
    pub shown: bool,
    /// It is the active part: the one a command works on when none is named, which always
    /// shows (decision 0033).
    pub active: bool,
    /// How it combines with the parts above it in its Live Boolean group (`part.set_operation`).
    pub operation: Operation,
    /// It starts a Live Boolean group.
    pub start: bool,
    /// Its paint shows (Show Paint, `part.set_paint_visible`): off, it draws in its material
    /// alone.
    pub paint_visible: bool,
}

/// The part list.
#[derive(Serialize, JsonSchema)]
pub struct Parts {
    /// Every part, in list order.
    pub parts: Vec<PartRow>,
}

impl Command for PartList {
    const NAME: &'static str = "part.list";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = Parts;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("list a sphere and a cube", Self {})
                .after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Parts, CommandError> {
        let doc = cx.doc.document();
        let showing = doc.showing();
        let parts = (doc.list().into_iter().enumerate())
            .map(|(index, (name, part))| PartRow {
                id: part.props().id.unwrap_or_default(),
                index: index as u64,
                name: name.to_string(),
                vertices: part.store().vertex_count() as u64,
                faces: part.store().face_count() as u64,
                rank: part.props().rank,
                visible: !part.props().hidden,
                isolate: showing.isolation.mark(part),
                shown: part.shown(showing),
                active: showing.active.is_some() && part.props().id == showing.active,
                operation: part.props().operation,
                start: part.props().start,
                paint_visible: !part.props().paint_hidden,
            })
            .collect();
        Ok(Parts { parts })
    }
}
register_command!(PartList, "List parts");
