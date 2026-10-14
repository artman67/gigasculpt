//! `ui.curve`: the curve the window holds for the curve editor's Copy and Paste (decision 0048,
//! section 4).

use gs_core::curve::Curve;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::no_window;
use crate::sculpt::curve_schema;
use crate::{Command, Ctx, Example, register_command};

/// Hold a curve in the window, as the curve editor's Copy does, to paste into another curve:
/// its points, their kinds and its settings. Paste writes it with that curve's own command
/// (`brush.set`), which is logged with the curve itself, so holding one changes nothing in the
/// session and is not logged. Left out, it keeps the one held. Answers the curve held. An agent
/// needs none of it: it reads a curve (`brush.describe`) and writes it where it wants.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiCurve {
    /// The curve to hold, as `brush.set` takes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "curve_schema")]
    pub copy: Option<Curve>,
}

/// The curve the window holds to paste.
#[derive(Clone, Serialize, JsonSchema)]
pub struct HeldCurve {
    /// Left out until a curve is copied.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "curve_schema")]
    pub copied: Option<Curve>,
}

impl Command for UiCurve {
    const NAME: &'static str = "ui.curve";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const LOGGED: bool = false;
    type Output = HeldCurve;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "hold a soft curve to paste (needs the window)",
                json!({ "copy": { "points": [[0, 0], [0.5, 0.8], [1, 1]], "smooth": true } }),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<HeldCurve, CommandError> {
        cx.window.with(|w| w.curve(&self)).ok_or_else(no_window)
    }
}
register_command!(UiCurve, "Hold a curve to paste");
