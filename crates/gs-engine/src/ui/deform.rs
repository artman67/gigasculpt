//! `ui.deform`: Header > Deform's X, Y and Z row and its Mask switch (decision 0041,
//! amended 2026-10-12, step `DF7a`; in the header since D1).

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::no_window;
use crate::{Command, Ctx, Example, register_command};

/// Light or unlight the Deform section's axes, or switch its Mask: which of the part's own X, Y
/// and Z its sliders and Flip mirror work along, and whether Repeat on other Parts keeps each
/// part's mask. The window's own settings, not logged, held for the session; what is left out
/// stays. Answers all four. An agent needs none of it: the section's controls are
/// `transform.apply` and `mesh.flip` calls, which name their axes and mask themselves.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiDeform {
    /// Light the part's own X axis (true) or not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<bool>,
    /// Light its Y axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<bool>,
    /// Light its Z axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<bool>,
    /// Repeat on other Parts keeps each part's own mask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<bool>,
}

/// The Deform section's settings.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, each one ui.deform sets on its own"
)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, JsonSchema)]
pub struct DeformSettings {
    /// X is lit.
    pub x: bool,
    /// Y is lit.
    pub y: bool,
    /// Z is lit.
    pub z: bool,
    /// Repeat on other Parts keeps each part's mask.
    pub mask: bool,
}

impl Command for UiDeform {
    const NAME: &'static str = "ui.deform";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    const LOGGED: bool = false;
    type Output = DeformSettings;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("light Z as well (needs the window)", json!({ "z": true }))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<DeformSettings, CommandError> {
        cx.window.with(|w| w.deform(&self)).ok_or_else(no_window)
    }
}
register_command!(UiDeform, "Pick the Deform section's axes");
