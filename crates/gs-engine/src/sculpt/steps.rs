//! Strength and Falloff steps (decision 0052, section 10): `brush.set`'s `strength_step` and
//! `falloff_step` move the brush's value by the session's increments, held to its range, and
//! `strength_increment` and `falloff_increment` set the increments, the same for every brush.

use gs_protocol::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{BrushSet, BrushSettings};
use crate::errors;

/// How far one step moves Strength and Falloff, for every brush. Its fields are named as
/// `brush.set`'s arguments, and `session.state` shows them under `brush.`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Increments {
    /// One Strength step, 0.01 to 0.5.
    pub strength_increment: f32,
    /// One Falloff step, 1 to 50.
    pub falloff_increment: f32,
}

impl Default for Increments {
    /// A twentieth of Strength's range and a twentieth of Falloff's.
    fn default() -> Self {
        Self {
            strength_increment: 0.05,
            falloff_increment: 10.0,
        }
    }
}

/// The increments' ranges.
const STRENGTH: [f32; 2] = [0.01, 0.5];
const FALLOFF: [f32; 2] = [1.0, 50.0];

/// Takes `set`'s increments into `next` and turns its steps into the values they reach from the
/// brush in effect's own, so the rest of `brush.set` changes them as it changes a typed value.
/// Refused, with nothing changed, for an increment out of range or a step given with its value.
pub(super) fn resolve(set: &mut BrushSet, next: &mut BrushSettings) -> Result<(), CommandError> {
    let given = [
        ("strength_increment", set.strength_increment, STRENGTH),
        ("falloff_increment", set.falloff_increment, FALLOFF),
    ];
    for (name, value, [low, high]) in given {
        if value.is_some_and(|v| !(low..=high).contains(&v)) {
            let message = format!("{name} must be from {low} to {high}");
            return Err(errors::args_invalid(&format!("/{name}"), message));
        }
    }
    let both = [
        (
            "strength",
            set.strength.is_some() && set.strength_step.is_some(),
        ),
        (
            "falloff",
            set.falloff.is_some() && set.falloff_step.is_some(),
        ),
    ];
    if let Some((name, _)) = both.iter().find(|b| b.1) {
        let message = format!("give {name} or {name}_step, not both");
        return Err(errors::args_invalid(&format!("/{name}_step"), message));
    }
    let by = &mut next.increments;
    by.strength_increment = set.strength_increment.unwrap_or(by.strength_increment);
    by.falloff_increment = set.falloff_increment.unwrap_or(by.falloff_increment);
    let by = *by;
    if set.strength_step.is_none() && set.falloff_step.is_none() {
        return Ok(());
    }
    let preset = next.resolved(&set.target(next)?)?;
    let stepped = |now: f32, steps: i32, by: f32, [low, high]: [f32; 2]| {
        let to = (now + steps as f32 * by).clamp(low, high);
        // Whole ten-thousandths, so steps up and back down land where they started.
        (to * 1e4).round() / 1e4
    };
    if let Some(steps) = set.strength_step {
        let to = stepped(preset.strength, steps, by.strength_increment, [0.0, 1.0]);
        set.strength = Some(to);
    }
    if let Some(steps) = set.falloff_step {
        let to = stepped(
            preset.tip.focal,
            steps,
            by.falloff_increment,
            [-100.0, 100.0],
        );
        set.falloff = Some(to);
    }
    Ok(())
}
