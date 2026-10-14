//! `group.list`.

use gs_core::groups::{fresh_after, group_color};
use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::largest;
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, register_command};

/// List a part's face groups: each id with its visible and hidden face counts and its color,
/// the largest id, and the id a new group gets next.
///
/// Counts are of the level you see. Colors come from the id alone, so they are the same in
/// every session.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupList {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// One face group.
#[derive(Serialize, JsonSchema)]
pub struct GroupInfo {
    /// Its id. 0 holds the faces in no group.
    pub id: u32,
    /// Its visible faces.
    pub visible: u64,
    /// Its hidden faces.
    pub hidden: u64,
    /// Its display color, `"#rrggbb"`. Group 0 is white: no tint.
    pub color: String,
}

/// A part's face groups.
#[derive(Serialize, JsonSchema)]
pub struct GroupsListed {
    /// The part.
    pub part: String,
    /// Every id a face holds, ascending.
    pub groups: Vec<GroupInfo>,
    /// The largest id a face holds, on any detail level.
    pub largest: u32,
    /// The id the next new group gets. Left out when no id is left: the largest is 2147483647.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<u32>,
}

impl Command for GroupList {
    const NAME: &'static str = "group.list";
    const KIND: Kind = Kind::Query;
    const PART: PartArg = PartArg::Existing;
    type Output = GroupsListed;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("list a cube's six sides", Self { part: None })
                .after("primitive.cube", json!({}))
                .after("group.make", json!({ "maker": { "by": "normals" } })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<GroupsListed, CommandError> {
        let name = pinned(self.part)?;
        let part = cx.doc.get_part(&name)?;
        let groups = (part.store().group_counts().into_iter())
            .map(|g| {
                let [r, gr, b, _] = group_color(g.id, 0);
                GroupInfo {
                    id: g.id,
                    visible: g.visible,
                    hidden: g.hidden,
                    color: format!("#{r:02x}{gr:02x}{b:02x}"),
                }
            })
            .collect();
        let largest = largest(part);
        Ok(GroupsListed {
            part: name,
            groups,
            largest,
            next: fresh_after(largest, 1),
        })
    }
}
register_command!(GroupList, "List face groups");
