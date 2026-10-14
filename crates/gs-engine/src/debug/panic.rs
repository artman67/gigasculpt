//! `debug.panic`.

use gs_protocol::{CommandError, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, register_command};

/// Crash on purpose in the middle of an edit, to check that the engine survives a bug.
///
/// It moves every vertex of the part to (1, 2, 3), then panics before finishing. The call
/// fails with `E_INTERNAL`, the engine keeps running, and the document is exactly as it was.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DebugPanic {
    #[doc = crate::command::part_doc!("The part to move before crashing.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

impl Command for DebugPanic {
    const NAME: &'static str = "debug.panic";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = ();

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("crash while moving a cube", Self { part: None })
                .after("primitive.cube", json!({}))
                .fails_with(ErrorCode::Internal),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<(), CommandError> {
        let part = pinned(self.part)?;
        let mut store = cx.doc.write_part(&part)?;
        let nodes = store.all_nodes();
        store
            .write_positions(&nodes)
            .each(|node| node.positions().fill(gs_core::Vec3::new(1.0, 2.0, 3.0)));
        panic!("debug.panic: crashed on purpose after moving part '{part}'");
    }
}
register_command!(DebugPanic, "Crash on purpose");
