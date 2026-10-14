//! `group.merge`.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Grouped, given};
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, register_command};

/// Fold face groups into one: every face of the `sources` groups, hidden faces included, moves
/// to the `target` group. No other face changes.
///
/// On a part with detail levels it works on the top level and shows the result on the level
/// you see.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupMerge {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The groups to fold in.
    #[schemars(length(min = 1))]
    pub sources: Vec<u32>,
    /// The group they become, at most 2147483647 (0 takes their faces out of every group).
    pub target: u32,
}

impl Command for GroupMerge {
    const NAME: &'static str = "group.merge";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    // Changes something only on a part with face groups, which random plans make too seldom to
    // count on: its claims are checked on plans that start by grouping each part by normals.
    const SETUP: &'static str =
        r#"[{"command": "group.make", "args": {"maker": {"by": "normals"}}}]"#;
    type Output = Grouped;

    fn examples() -> Vec<Example<Self>> {
        let merge = GroupMerge {
            part: None,
            sources: vec![2, 3],
            target: 5,
        };
        vec![
            Example::new("merge groups 2 and 3 into group 5", merge)
                .after("primitive.cube", json!({}))
                .after("group.make", json!({ "maker": { "by": "normals" } })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Grouped, CommandError> {
        let part = pinned(self.part)?;
        cx.noop_reason("no face is in a source group");
        let (sources, target) = (self.sources, given("/target", self.target)?);
        let mut changed = false;
        crate::level::whole_part(cx, &part, |s| {
            let nodes = s.merge_groups(&sources, target);
            changed = !nodes.is_empty();
            nodes
        })?;
        let ids = changed.then_some([target, target]);
        Ok(Grouped::of(&part, cx.doc.get_part(&part)?, ids))
    }
}
register_command!(GroupMerge, "Merge face groups");
