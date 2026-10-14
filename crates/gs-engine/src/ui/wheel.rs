//! `ui.wheel`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{WheelInfo, no_window};
use crate::{Command, Ctx, Example, errors, register_command};

/// Open the brush wheel at the pointer, or let go of it; or put a brush in one of its slots.
///
/// The wheel is one ring of the layout's brush controls around the pointer, the first at the
/// top and the rest clockwise, a little away from the hand that holds the pen (`ui.settings`'
/// `wheel_offset`). Letting go (`open: false`) picks the control the pointer has moved toward,
/// if it has moved out of the wheel's middle, or the one a letter jumped to, and runs its
/// command; otherwise it just closes. Holding the wheel key opens it and releasing the key lets
/// go, so a flick toward a brush picks it in one gesture; clicking a slot picks it too. `slot`
/// with `brush` puts that brush in the slot (a right click on a slot puts the brush in use
/// there), kept with the window's layout state; `restore` puts the layout's own slots back.
/// Answers whether it is open, what it picked and its slots.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiWheel {
    /// True to open it at the pointer, false to let go of it; left out, it stays as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    /// The slot to put `brush` in: 0 is the top one, then clockwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// The brush for `slot`, by name (`brush.list`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<String>,
    /// True to put the layout's own slots back, before any `slot`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restore: bool,
}

impl Command for UiWheel {
    const NAME: &'static str = "ui.wheel";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// What it picks runs as its own command, which is logged; the wheel itself is not.
    const LOGGED: bool = false;
    type Output = WheelInfo;

    fn examples() -> Vec<Example<Self>> {
        let open = Self {
            open: Some(true),
            slot: None,
            brush: None,
            restore: false,
        };
        vec![
            Example::new("open the wheel (needs the window)", open)
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<WheelInfo, CommandError> {
        if self.slot.is_some() != self.brush.is_some() {
            let message = "`slot` and `brush` go together: the brush to put in the slot";
            return Err(errors::args_invalid("/brush", message));
        }
        let wheel = cx.window.with(|w| w.wheel(&self)).ok_or_else(no_window)?;
        wheel.map_err(|e| errors::error(ErrorCode::NotFound, e))
    }
}
register_command!(UiWheel, "Brush wheel");
