//! `ui.adjust`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::no_window;
use crate::{Command, Ctx, Example, register_command};

/// Show the adjust panel again, or close it (decision 0060, section 5, as amended 2026-10-13,
/// step H2), as F9 and the panel's × do. After a command run from a header menu, the palette or
/// a key makes one new undo step, the panel in the 3D view's lower corner shows the step's
/// arguments; a change undoes the step and runs it again with the new value, as one step. It
/// shows again only while its step is the newest undo step; otherwise the status line says
/// "Nothing to adjust". Left out, `open` changes nothing. Answers whether the panel shows, and
/// its command and step id while it does.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiAdjust {
    /// True to show the panel again for the newest step, false to close it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
}

/// The adjust panel after `ui.adjust`.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct AdjustInfo {
    /// Whether it shows.
    pub open: bool,
    /// The command of the step it shows; null while it is closed.
    pub command: Option<String>,
    /// The id of the step it shows, as `history.list` gives it; null while it is closed.
    pub step: Option<u64>,
}

impl Command for UiAdjust {
    const NAME: &'static str = "ui.adjust";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes what the window shows, not the session; what the panel runs is logged.
    const LOGGED: bool = false;
    type Output = AdjustInfo;

    fn examples() -> Vec<Example<Self>> {
        let open = |open| Self { open: Some(open) };
        vec![
            Example::new("show the adjust panel again (needs the window)", open(true))
                .fails_with(ErrorCode::Precondition),
            Example::new("close the adjust panel (needs the window)", open(false))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<AdjustInfo, CommandError> {
        cx.window.with(|w| w.adjust(&self)).ok_or_else(no_window)
    }
}
register_command!(UiAdjust, "Adjust panel");
