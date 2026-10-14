//! `ui.describe`.

use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::UiDescription;
use crate::{Command, Ctx, Example, register_command};

/// Describe the window: its panels and their widgets, with the command each widget runs, its
/// key and whether it can be used now, and each tray's sections with whether they are folded;
/// the pen cursor; the 3D view; frame timing and pen latency hop by hop.
///
/// In a session without a window, only the cursor is set.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiDescribe {}

impl Command for UiDescribe {
    const NAME: &'static str = "ui.describe";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = UiDescription;

    fn examples() -> Vec<Example<Self>> {
        vec![Example::new("describe the window", Self {})]
    }

    fn run(self, cx: &mut Ctx) -> Result<UiDescription, CommandError> {
        super::gizmo::show(cx);
        Ok(cx.window.with(super::WindowHost::describe))
    }
}
register_command!(UiDescribe, "Describe window");
