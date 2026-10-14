//! `level.delete_higher` and `level.delete_lower`.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{LevelState, change, check_thawed, subdivided_cube};
use crate::command::PartArg;
use crate::document::pinned;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Delete every detail level above the active one: the active level becomes the top, edits not
/// carried up yet included. One undo step. With one level left the part is a plain part again.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelDeleteHigher {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// Delete every detail level below the active one: the active level becomes the base (level
/// 1), and the levels above keep their numbering. One undo step. With one level left the part
/// is a plain part again.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelDeleteLower {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
}

/// Deletes the levels above the active one (`higher`) or below it.
fn delete(cx: &mut Ctx, part: Option<PartRef>, higher: bool) -> Result<LevelState, CommandError> {
    let name = pinned(part)?;
    let why = if higher {
        "the top is active, there is no level above"
    } else {
        "the base is active, there is no level below"
    };
    change(cx, &name, why, |l| {
        let done = if higher {
            l.delete_higher()
        } else {
            l.delete_lower()
        };
        Ok(done.then_some(0))
    })
}

impl Command for LevelDeleteHigher {
    const NAME: &'static str = "level.delete_higher";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = LevelState;

    fn examples() -> Vec<Example<Self>> {
        let base = subdivided_cube(
            "keep only the base of a subdivided cube",
            Self { part: None },
        );
        vec![base.after("level.set", json!({"level": 1}))]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        check_thawed(cx, self.part.as_ref(), Self::NAME)
    }

    fn run(self, cx: &mut Ctx) -> Result<LevelState, CommandError> {
        delete(cx, self.part, true)
    }
}
register_command!(LevelDeleteHigher, "Delete higher levels");

impl Command for LevelDeleteLower {
    const NAME: &'static str = "level.delete_lower";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = LevelState;

    fn examples() -> Vec<Example<Self>> {
        let top = Self { part: None };
        vec![subdivided_cube(
            "make a subdivided cube's top its base",
            top,
        )]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        check_thawed(cx, self.part.as_ref(), Self::NAME)
    }

    fn run(self, cx: &mut Ctx) -> Result<LevelState, CommandError> {
        delete(cx, self.part, false)
    }
}
register_command!(LevelDeleteLower, "Delete lower levels");
