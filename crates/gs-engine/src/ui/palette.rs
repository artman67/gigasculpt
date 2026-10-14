//! `ui.palette`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, Violation};
use jsonschema::error::ValidationErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{PaletteInfo, no_window};
use crate::errors::{args_invalid, did_you_mean, internal};
use crate::{Command, Ctx, Example, register_command};

/// Open or close the command palette: fuzzy search over every command and every control of the
/// layout shown, showing each one's docs, key and where it lives, and running the one picked.
/// A command that needs arguments asks for them in a form first.
///
/// The palette key opens it in the window; this is the same action for agents and keymaps.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiPalette {
    /// True to open it, false to close it. Left out, true.
    #[serde(default = "crate::command::yes")]
    pub open: bool,
    /// Text to type into it. Left out, it keeps what is there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Open it asking for this command's arguments (for example `file.import`, which asks
    /// for a path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<String>,
    /// With `form`: the arguments the command has already (for example `{"by": "shells"}` for
    /// `part.split`), so the form asks only for the rest. Each must be one the command takes,
    /// with a value it accepts.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
    /// With `form`: optional arguments it asks for too, besides the required ones (for
    /// example `["file"]` for `reference.open`). An argument is given in `args` or asked for,
    /// not both.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ask: Vec<String>,
    /// True to quit: the window closes as its close button closes it, asking first in the
    /// palette while there are unsaved changes (File > Quit, Ctrl+Q).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quit: bool,
}

impl Command for UiPalette {
    const NAME: &'static str = "ui.palette";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes what the window shows, not the session: a replay never needs it.
    const LOGGED: bool = false;
    type Output = PaletteInfo;

    fn examples() -> Vec<Example<Self>> {
        let open = Self {
            open: true,
            query: Some("sphere".into()),
            form: None,
            args: Map::new(),
            ask: Vec::new(),
            quit: false,
        };
        vec![
            Example::new("search for sphere (needs the window)", open)
                .fails_with(ErrorCode::Precondition),
        ]
    }

    /// Checks the form against its command here, so a dry run (the window's, for each of its
    /// controls) refuses a mistake in a layout file before anyone presses the control.
    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        let Some(form) = &self.form else {
            if self.args.is_empty() && self.ask.is_empty() {
                return Ok(());
            }
            return Err(wrong("/args", "args and ask go with form".into()));
        };
        let names = cx.catalog.commands.iter().map(|c| c.name.as_str());
        let Some(info) = cx.catalog.commands.iter().find(|c| c.name == *form) else {
            return Err(CommandError {
                suggestions: did_you_mean(form, names),
                ..wrong("/form", format!("there is no command named '{form}'"))
            });
        };
        let schema = &info.args_schema;
        let takes = |name: &str| schema["properties"].get(name).is_some();
        if let Some(name) = self.ask.iter().find(|n| !takes(n)) {
            return Err(wrong("/ask", format!("{form} has no argument '{name}'")));
        }
        if let Some(name) = self.ask.iter().find(|n| self.args.contains_key(*n)) {
            let message = format!("'{name}' is given in args and asked for: give it or ask");
            return Err(wrong("/ask", message));
        }
        let validator = jsonschema::validator_for(schema).map_err(|e| internal(&e.to_string()))?;
        let given = Value::Object(self.args.clone());
        // What the form asks for is missing until it is typed: only a missing argument at the
        // top is left to the form.
        let left_to_form = |e: &jsonschema::ValidationError| {
            matches!(e.kind(), ValidationErrorKind::Required { .. })
                && e.instance_path().to_string().is_empty()
        };
        match validator.iter_errors(&given).find(|e| !left_to_form(e)) {
            Some(e) => {
                let path = format!("/args{}", e.instance_path());
                Err(wrong(&path, format!("{form}: {e}")))
            }
            None => Ok(()),
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<PaletteInfo, CommandError> {
        let shown = cx.window.with(|w| w.palette(&self)).ok_or_else(no_window)?;
        shown.map_err(|e| args_invalid("/form", e))
    }
}
register_command!(UiPalette, "Command palette");

/// `E_ARGS_INVALID` at JSON pointer `path`, listed as that argument's violation, as a schema's
/// would be.
pub(super) fn wrong(path: &str, message: String) -> CommandError {
    let mut refused = args_invalid(path, message.clone());
    refused.violations.push(Violation {
        path: path.into(),
        message,
    });
    refused
}
