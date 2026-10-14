//! Scene tray > Detail (decision 0060, section 7, as amended for S1): the `Level` group's two
//! arrows around the active part's level and count (`3 of 5`, `5 frozen` while its levels are
//! frozen, `none` in an empty scene), with no label so that `‹ 2 of 3 › Subdivide ···` fits one
//! row, then the set's controls with no group: Subdivide, the level's `···`, and Live with its
//! settings. An arrow at its end, and Delete lower or higher there, would change nothing, so the
//! window holds them back with the engine's words ([`ends`]).

use egui::RichText;
use gs_engine::SessionState;
use gs_protocol::Step;
use serde_json::{Value, json};

use super::{Fit, Now, control, controls, controls_of, label};
use crate::record::{Recorder, with_args};

/// The set's group whose arrows step the level.
const LEVEL: &str = "Level";

/// The `Level` group's arrows and readout, then the set's controls with no group, which go on
/// along the row.
pub(super) fn section(ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Option<Step> {
    let mut asked = None;
    for (i, c) in controls_of(&format!("detail/{LEVEL}"), now)
        .iter()
        .enumerate()
    {
        if i == 1 {
            label(ui, RichText::new(readout(now)), rec);
        }
        asked = control(ui, c, true, now, rec).or(asked);
    }
    controls(ui, "detail/", Fit::Section, false, now, rec).or(asked)
}

/// The active part's level and count, `3 of 5`; while its levels are frozen, how many,
/// `5 frozen`; `none` in an empty scene. Folded, Detail's title says it too (`summary`).
pub(super) fn readout(now: &Now) -> String {
    let n = |name: &str| now.state.get(name).and_then(Value::as_u64);
    match (n("parts.level"), n("parts.levels"), n("parts.frozen")) {
        (_, _, Some(frozen)) if frozen > 0 => format!("{frozen} frozen"),
        (Some(level), Some(count), _) => format!("{level} of {count}"),
        _ => "none".into(),
    }
}

/// The level steps, each with why it would change nothing with `state` as it is, or `None`
/// while it would change something: stepping down or deleting lower at the base, stepping up
/// or deleting higher at the top. Frozen levels and an empty scene hold none here, as the
/// engine refuses those itself.
pub(crate) fn ends(state: &SessionState) -> [(Step, Option<String>); 4] {
    let (down, up) = (end(state, true), end(state, false));
    [
        (with_args("level.step", json!({ "by": -1 })), down.clone()),
        (with_args("level.delete_lower", json!({})), down),
        (with_args("level.step", json!({ "by": 1 })), up.clone()),
        (with_args("level.delete_higher", json!({})), up),
    ]
}

/// Why going `down` (else up) from the active level would change nothing, if it would, in the
/// engine's words for that no-op (its twin: gs-engine `level/mod.rs` `change`).
fn end(state: &SessionState, down: bool) -> Option<String> {
    let (name, [count, level, frozen]) = (state.active.as_deref()?, state.levels?);
    let stuck = if down { level == 1 } else { level == count };
    if frozen > 0 || !stuck {
        return None;
    }
    let why = if down {
        "the base is active, there is no level below"
    } else {
        "the top is active, there is no level above"
    };
    Some(if count == 1 {
        format!("part '{name}' has one level: {why}")
    } else {
        format!("part '{name}': {why}")
    })
}
