//! `session.control`: the switches a human uses to keep agents out of the way.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Command, Ctx, Example, register_command};

/// Pause agents, mark a human gesture in progress, or let agents move the window's view. Only
/// the human at the window can use this.
///
/// While paused, every non-query call from an agent or script fails with `E_PAUSED`. While a
/// gesture (a pen stroke, a slider drag) is in progress, agent edits fail with `E_BUSY` and
/// should be retried. With `agents_move_view` off (the default) an agent's camera command on
/// `main` fails with `E_PRECONDITION` while the window shows it. Queries always work. Leave a
/// switch out to keep it as it is. Not written to the command log: it changes who may act, not
/// what is in the session.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionControl {
    /// True to pause agents, false to let them continue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    /// True when a human gesture starts, false when it ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<bool>,
    /// True to let agents move `main`, the window's view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents_move_view: Option<bool>,
}

/// The switches after the call.
#[derive(Serialize, JsonSchema)]
pub struct Switches {
    /// Agents are paused.
    pub paused: bool,
    /// A human gesture is in progress.
    pub gesture: bool,
    /// Agents may move `main`.
    pub agents_move_view: bool,
}

impl Command for SessionControl {
    const NAME: &'static str = "session.control";
    const KIND: Kind = Kind::Session;
    const COST: Cost = Cost::Instant;
    const HUMAN_ONLY: bool = true;
    const LOGGED: bool = false;
    type Output = Switches;

    fn examples() -> Vec<Example<Self>> {
        let pause = Self {
            paused: Some(true),
            gesture: None,
            agents_move_view: None,
        };
        let stroke = Self {
            paused: None,
            gesture: Some(true),
            agents_move_view: None,
        };
        vec![
            Example::new("pause agents", pause),
            Example::new("start a gesture", stroke),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Switches, CommandError> {
        let control = &mut *cx.control;
        control.paused = self.paused.unwrap_or(control.paused);
        control.gesture = self.gesture.unwrap_or(control.gesture);
        control.agents_move_view = self.agents_move_view.unwrap_or(control.agents_move_view);
        Ok(Switches {
            paused: control.paused,
            gesture: control.gesture,
            agents_move_view: control.agents_move_view,
        })
    }
}
register_command!(SessionControl, "Session control");
