//! `part.get`.

use std::sync::Arc;

use gs_protocol::{CommandError, Kind, PartRef, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::{Operation, pinned};
use crate::{Command, Ctx, Example, register_command};

/// Read a part's full mesh data: positions, faces and attribute columns.
///
/// Returns every vertex, so prefer `mesh.stats` or `mesh.bounds` for large parts.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartGet {
    #[doc = crate::command::part_doc!("The part to read.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// A part's mesh data, with its Live Boolean marks and its Show Paint switch beside it when they
/// are not the defaults, so what a part never marked gives is exactly what `part.create` takes.
#[derive(Serialize, JsonSchema)]
pub struct PartData {
    #[serde(flatten)]
    pub mesh: PlainMesh,
    /// How it combines with the parts above it in its Live Boolean group. Left out: `union`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<Operation>,
    /// It starts a Live Boolean group. Left out: it does not.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub start: bool,
    /// Its paint shows (`part.set_paint_visible`). Left out: it does.
    #[serde(skip_serializing_if = "Clone::clone")]
    pub paint_visible: bool,
}

impl Command for PartGet {
    const NAME: &'static str = "part.get";
    const KIND: Kind = Kind::Query;
    const PART: PartArg = PartArg::Existing;
    type Output = PartData;

    fn examples() -> Vec<Example<Self>> {
        let part = Some(PartRef("cube".into()));
        vec![Example::new("read a cube", Self { part }).after("primitive.cube", json!({}))]
    }

    fn run(self, cx: &mut Ctx) -> Result<PartData, CommandError> {
        let part = pinned(self.part)?;
        let props = cx.doc.get_part(&part)?.props();
        let mesh = cx.doc.part(&part).map(Arc::unwrap_or_clone)?;
        Ok(PartData {
            mesh,
            operation: (props.operation != Operation::Union).then_some(props.operation),
            start: props.start,
            paint_visible: !props.paint_hidden,
        })
    }
}
register_command!(PartGet, "Read part");
