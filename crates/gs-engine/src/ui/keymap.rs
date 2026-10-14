//! `ui.keymap`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::no_window;
use super::palette::wrong;
use crate::errors::did_you_mean;
use crate::{Command, Ctx, Example, register_command};

/// Bind a key to a command, or put keys back as they were built (decision 0025, section 5).
/// The window's keys are the built-in ones with the user's changes over them, and the user's
/// `keymap.toml` holds the changes only. `bind` names a key as that file writes it (`Ctrl+K`,
/// the sequence `B S T`, the wheel notch `Alt+WheelUp`) and `command` what it runs, with
/// `args`; an empty `command` unbinds the key. A key another binding holds, or one that starts
/// a sequence another holds or that another starts, is refused naming each holder until
/// `replace` is true. `restore` puts one key back as built and `restore_defaults` every key.
/// With none of these it changes nothing. Answers with the changes and every binding after.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiKeymap {
    /// The key to bind, as the keymap file writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    /// With `bind`: the command the key runs, or empty to unbind the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// With `bind`: the command's arguments. One written `"drag"` is set by holding the key
    /// and moving the pointer sideways; one written `"held"` is true while the key is down.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
    /// With `bind`: true to take the key from the bindings that hold it, which are dropped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace: bool,
    /// A key to put back as built, by dropping the change to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<String>,
    /// True to put every key back as built, dropping every change.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restore_defaults: bool,
}

/// A key and what it runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
pub struct KeyBinding {
    /// The key as the keymap file writes it.
    pub key: String,
    /// The command it runs; empty in a change that unbinds the key.
    pub command: String,
    /// The command's arguments.
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
}

/// The keys after `ui.keymap`.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct KeymapInfo {
    /// The user's changes to the built-in keys, as `keymap.toml` holds them.
    pub changes: Vec<KeyBinding>,
    /// Every binding in force: the built-in ones with the changes over them.
    pub bindings: Vec<KeyBinding>,
}

impl Command for UiKeymap {
    const NAME: &'static str = "ui.keymap";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    /// It changes the user's keymap file, not the session.
    const LOGGED: bool = false;
    type Output = KeymapInfo;

    fn examples() -> Vec<Example<Self>> {
        let bind = |command: &str| Self {
            bind: Some("Ctrl+K".into()),
            command: Some(command.into()),
            ..Self::default()
        };
        let cube = bind("primitive.cube");
        let unknown = bind("primitive.cub");
        vec![
            Example::new("bind Ctrl+K to a new cube (needs the window)", cube)
                .fails_with(ErrorCode::Precondition),
            Example::new(
                "a command that does not exist is refused, window or not",
                unknown,
            )
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    /// Checks the arguments against each other and the command against the catalog, so a dry
    /// run refuses a mistake before the window sees it.
    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        let asked = [
            self.bind.is_some(),
            self.restore.is_some(),
            self.restore_defaults,
        ];
        if asked.into_iter().filter(|a| *a).count() > 1 {
            let path = if self.restore_defaults {
                "/restore_defaults"
            } else {
                "/restore"
            };
            let message = "give one of bind, restore and restore_defaults";
            return Err(wrong(path, message.into()));
        }
        if self.bind.is_none() {
            let stray = [
                ("/command", self.command.is_some()),
                ("/args", !self.args.is_empty()),
                ("/replace", self.replace),
            ];
            return match stray.into_iter().find(|(_, given)| *given) {
                Some((path, _)) => Err(wrong(path, format!("{} goes with bind", &path[1..]))),
                None => Ok(()),
            };
        }
        let Some(command) = &self.command else {
            let message = "bind needs a command; an empty one unbinds the key";
            return Err(wrong("/command", message.into()));
        };
        let commands = &cx.catalog.commands;
        if command.is_empty() || commands.iter().any(|c| c.name == *command) {
            return Ok(());
        }
        let names = commands.iter().map(|c| c.name.as_str());
        Err(CommandError {
            suggestions: did_you_mean(command, names),
            ..wrong("/command", format!("there is no command named '{command}'"))
        })
    }

    fn run(self, cx: &mut Ctx) -> Result<KeymapInfo, CommandError> {
        cx.window.with(|w| w.keymap(&self)).ok_or_else(no_window)?
    }
}
register_command!(UiKeymap, "Keymap");
