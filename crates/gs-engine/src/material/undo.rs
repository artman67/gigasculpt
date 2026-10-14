//! `material.undo` and `material.redo` (decision 0043, section 5): the palette's own history,
//! apart from the scene's. They never touch the sculpt, and `history.undo` never touches them.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::hand_over;
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// The palette after `material.undo` or `material.redo`.
#[derive(Serialize, JsonSchema)]
pub struct PaletteStepped {
    /// The command whose step was undone or redone, for example `material.rename`.
    pub command: String,
    /// The client that made the step.
    pub by: String,
    /// The active material's name.
    pub active: String,
    /// The step took the active material away, so the first material is active now.
    pub active_removed: bool,
    /// Reference Views whose own look followed a material: to its name now, or back to the
    /// main view's look when it went away.
    pub references: Vec<String>,
}

/// Take back the palette's last step: a copy, a rename, a delete or an edit (up to 50 back). Choosing a
/// material is not a step. The sculpt never changes. An agent takes back only its own steps.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialUndo {}

/// Make the last step `material.undo` took back again. A new palette step clears what can be
/// redone.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterialRedo {}

/// Says why there is no step to take, so the window can grey its button.
fn can_step(cx: &Ctx, forward: bool) -> Result<(), Unavailable> {
    if cx.palette.can_travel(forward) {
        return Ok(());
    }
    let (what, fix) = if forward {
        ("redo", "material.undo takes a step back that this redoes")
    } else {
        (
            "undo",
            "material.copy, material.rename, material.delete and material.set make palette steps",
        )
    };
    Err(Unavailable::new(format!("the palette has nothing to {what}")).fix(fix))
}

/// The examples of `material.undo` (`forward` false) and `material.redo`: a copy taken back
/// (and made again), and nothing to step to.
fn examples<T: Serialize>(forward: bool) -> Vec<Example<T>> {
    let (stepped, missing) = if forward {
        ("the copy again", "nothing to redo")
    } else {
        ("take a copy back", "nothing to take back")
    };
    let mut stepped = Example::raw(stepped, json!({})).after("material.copy", json!({}));
    if forward {
        stepped = stepped.after("material.undo", json!({}));
    }
    let missing = Example::raw(missing, json!({})).fails_with(ErrorCode::Precondition);
    vec![stepped, missing]
}

/// One step through the palette's history.
fn step(cx: &mut Ctx, forward: bool) -> Result<PaletteStepped, CommandError> {
    let before = cx.palette.clone();
    let moved = cx.palette.travel(forward, cx.origin)?;
    let active = cx.palette.now.active().id;
    let references = hand_over(cx, before.clone())?;
    Ok(PaletteStepped {
        command: moved.command.to_string(),
        by: moved.client,
        active: cx.palette.now.active().name.clone(),
        active_removed: active != before.now.active,
        references,
    })
}

impl Command for MaterialUndo {
    const NAME: &'static str = "material.undo";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = PaletteStepped;

    fn examples() -> Vec<Example<Self>> {
        examples(false)
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        can_step(cx, false)
    }

    fn run(self, cx: &mut Ctx) -> Result<PaletteStepped, CommandError> {
        step(cx, false)
    }
}
register_command!(MaterialUndo, "Undo material");

impl Command for MaterialRedo {
    const NAME: &'static str = "material.redo";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = PaletteStepped;

    fn examples() -> Vec<Example<Self>> {
        examples(true)
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        can_step(cx, true)
    }

    fn run(self, cx: &mut Ctx) -> Result<PaletteStepped, CommandError> {
        step(cx, true)
    }
}
register_command!(MaterialRedo, "Redo material");
