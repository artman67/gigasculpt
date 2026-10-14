//! `level.set` and `level.step`: switching the active detail level.

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{LevelState, change, check_thawed, subdivided_cube};
use crate::command::PartArg;
use crate::document::pinned;
use crate::errors;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Make another detail level of a part the one edited and drawn.
///
/// Edits made on the level left behind are carried up to the top first, keeping the finer
/// levels' detail in frames that turn with the edited surface; points the edit could not reach
/// keep their exact values. One undo step, so undo puts back the level and its edits exactly.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelSet {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The level, from 1 (the base) to the part's level count (the top).
    #[schemars(range(min = 1))]
    pub level: u32,
}

/// Step a part's active detail level up or down by `by` levels, stopping at the base and the
/// top. Carries edits up like `level.set`. One undo step.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelStep {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Levels to move: 1 steps up (denser), -1 down.
    pub by: i32,
}

impl Command for LevelSet {
    const NAME: &'static str = "level.set";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = LevelState;

    fn examples() -> Vec<Example<Self>> {
        let base = Self {
            part: None,
            level: 1,
        };
        vec![subdivided_cube(
            "go back to the base of a subdivided cube",
            base,
        )]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        check_thawed(cx, self.part.as_ref(), Self::NAME)
    }

    fn run(self, cx: &mut Ctx) -> Result<LevelState, CommandError> {
        let name = pinned(self.part)?;
        let part = cx.doc.get_part(&name)?;
        let count = part.levels().map_or(1, gs_core::Levels::count);
        if self.level == 0 || self.level as usize > count {
            return Err(errors::args_invalid(
                "/level",
                format!("part '{name}' has levels 1 to {count}"),
            ));
        }
        let why = format!("level {} is active already", self.level);
        let j = self.level as usize - 1;
        change(cx, &name, &why, |l| {
            if l.active() == j {
                return Ok(None);
            }
            l.set(j).map(|c| Some(c.points))
        })
    }
}
register_command!(LevelSet, "Go to level");

impl Command for LevelStep {
    const NAME: &'static str = "level.step";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    type Output = LevelState;

    fn examples() -> Vec<Example<Self>> {
        let down = Self { part: None, by: -1 };
        vec![subdivided_cube(
            "step a subdivided cube down one level",
            down,
        )]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        check_thawed(cx, self.part.as_ref(), Self::NAME)
    }

    fn run(self, cx: &mut Ctx) -> Result<LevelState, CommandError> {
        let name = pinned(self.part)?;
        let why = if self.by < 0 {
            "the base is active, there is no level below"
        } else {
            "the top is active, there is no level above"
        };
        change(cx, &name, why, |l| {
            let to = if self.by < 0 {
                l.active().saturating_sub(self.by.unsigned_abs() as usize)
            } else {
                (l.active() + self.by as usize).min(l.top_level())
            };
            if to == l.active() {
                return Ok(None);
            }
            l.set(to).map(|c| Some(c.points))
        })
    }
}
register_command!(LevelStep, "Step level");
