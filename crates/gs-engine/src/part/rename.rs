//! `part.rename` and `part.reorder` (decision 0028, section 3).

use gs_protocol::{CommandError, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{NewPart, refused_name, report};
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// Give a part a new name. Its id, place and everything else stay.
///
/// A name is any text that does not start with `#` or `@` (those name parts by id and place)
/// and that no other part has.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartRename {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Its new name.
    #[schemars(length(min = 1))]
    pub name: String,
}

impl Command for PartRename {
    const NAME: &'static str = "part.rename";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("name the only part head", json!({ "name": "head" }))
                .after("primitive.sphere", json!({})),
            Example::raw(
                "rename the second part by its place",
                json!({ "part": "@1", "name": "body" }),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({})),
            Example::raw(
                "a name another part has is refused",
                json!({ "part": "sphere", "name": "cube" }),
            )
            .after("primitive.sphere", json!({}))
            .after("primitive.cube", json!({}))
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        let same = self.part.as_ref().is_some_and(|p| p.0 == self.name);
        match refused_name(cx, &self.name) {
            Some(reason) if !same => Err(reason),
            _ => Ok(()),
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let part = pinned(self.part)?;
        if part == self.name {
            cx.noop_reason(format!("the part is already named '{part}'"));
        } else {
            cx.doc.rename_part(&part, &self.name)?;
        }
        report(cx, &self.name)
    }
}
register_command!(PartRename, "Rename part");

/// Where `part.reorder` moves a part.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Move {
    /// One place up.
    Up,
    /// One place down.
    Down,
    /// To the top of the list.
    Top,
    /// To the bottom of the list.
    Bottom,
    /// To this place, from 0 (`{"place": 2}`).
    Place(u64),
}

/// Move a part up or down the part list. The other parts keep their order.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartReorder {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Where to.
    pub to: Move,
}

impl Command for PartReorder {
    const NAME: &'static str = "part.reorder";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        let three = |e: Example<Self>| {
            e.after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({}))
                .after("primitive.sphere", json!({ "name": "head" }))
        };
        vec![
            three(Example::raw(
                "move the first part down one place",
                json!({ "part": "sphere", "to": "down" }),
            )),
            three(Example::raw(
                "move the last part to the top",
                json!({ "part": "head", "to": "top" }),
            )),
            three(Example::raw(
                "move the first part to place 1",
                json!({ "part": "@0", "to": { "place": 1 } }),
            )),
            three(Example::raw(
                "a place past the end is refused",
                json!({ "part": "cube", "to": { "place": 3 } }),
            ))
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let part = pinned(self.part)?;
        let mut order = cx.doc.list_names();
        let last = order.len().saturating_sub(1);
        let at = (order.iter().position(|name| *name == part))
            .ok_or_else(|| errors::internal("a pinned part is not in the list"))?;
        let to = match self.to {
            Move::Up => at.saturating_sub(1),
            Move::Down => (at + 1).min(last),
            Move::Top => 0,
            Move::Bottom => last,
            Move::Place(n) if n as usize <= last => n as usize,
            Move::Place(n) => {
                let why = format!("place {n} is past the end: places run from 0 to {last}");
                return Err(errors::args_invalid("/to/place", why));
            }
        };
        if to == at {
            cx.noop_reason(format!("'{part}' is already at place {at}"));
        } else {
            let moved = order.remove(at);
            order.insert(to, moved);
            cx.doc.arrange(&order)?;
        }
        report(cx, &part)
    }
}
register_command!(PartReorder, "Reorder parts");
