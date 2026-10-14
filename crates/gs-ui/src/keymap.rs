//! The keymap: the built-in keys plus a plain file of the user's changes, binding keys to
//! commands. A key does nothing but run its command, so everything a key does, an agent can do
//! too. The built-in keys always apply (decision 0025); the file (`version = 2`) lists only the
//! bindings the user added or changed: one for a key that has a built-in binding replaces it,
//! and one with an empty `command` unbinds the key. A change naming a command the program does
//! not have is refused alone, and the status line names it. `ui.keymap` binds a key (naming
//! what holds a taken one until told to replace it) and puts one key or every key back,
//! writing the changes alone to the file.
//!
//! ```toml
//! version = 2
//!
//! [[bind]]
//! key = "Ctrl+Shift+Z"
//! command = "history.redo"
//!
//! [[bind]]
//! key = "B S T"
//! command = "brush.set"
//! args = { brush = "Draw" }
//!
//! [[bind]]
//! key = "S"
//! command = "brush.set"
//! args = { size_px = "drag" }
//! ```
//!
//! Keys are egui key names (`Space`, `Z`, `F1`, `Escape`, `ArrowUp`, `[`) with `Ctrl+`,
//! `Shift+`, `Alt+` and `Super+` in front, in any order; `WheelUp` and `WheelDown` are a notch
//! of the wheel, bound alone (`Alt+WheelUp`). Several keys separated by spaces are a sequence,
//! pressed one after another. An argument written as `"drag"` is set by holding
//! the key and moving the pointer sideways; the command runs once, when the key is let go. An
//! argument written as `"held"` is true while the key is down: the command runs when it goes
//! down and again, with false, when it comes up.

mod binding;
mod brush;
mod built_in;
mod file;

use binding::parse_keys;
pub use binding::{Binding, Chord, Press, Pressed};
pub(crate) use brush::picked;
pub use brush::{BrushKeys, NO_KEY};
use built_in::{BUILT_IN, YOURS};
pub(crate) use file::write_file;

/// Every binding: the built-in ones with the user's changes over them.
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    pub bindings: Vec<Binding>,
    /// The user's changes, as the keymap file lists them.
    pub changes: Vec<Binding>,
}

impl Keymap {
    /// The binding for `chord` pressed alone.
    pub fn lookup(&self, chord: &Chord) -> Option<&Binding> {
        match self.sequence(std::slice::from_ref(chord)) {
            Pressed::Binding(b) => Some(b),
            Pressed::Prefix | Pressed::Nothing => None,
        }
    }

    /// What `chords`, pressed one after another, mean.
    pub fn sequence(&self, chords: &[Chord]) -> Pressed<'_> {
        let mut prefix = false;
        for b in &self.bindings {
            let Ok(keys) = parse_keys(&b.key) else {
                continue;
            };
            if keys == chords {
                return Pressed::Binding(b);
            }
            prefix |= keys.starts_with(chords);
        }
        if prefix {
            Pressed::Prefix
        } else {
            Pressed::Nothing
        }
    }

    /// The bindings in force by context, as the keys sheet lists them: each built-in context
    /// with the keys it holds (a key the user rebound stays in its context), then `Your` with
    /// the keys only the user's changes bind.
    pub fn by_context(&self) -> Vec<(&'static str, Vec<&Binding>)> {
        let held: Vec<Vec<Vec<Chord>>> = (BUILT_IN.iter())
            .map(|(_, built)| {
                (built.iter())
                    .filter_map(|b| parse_keys(&b.key).ok())
                    .collect()
            })
            .collect();
        let mut lists: Vec<(&str, Vec<&Binding>)> = BUILT_IN
            .iter()
            .map(|(name, _)| (*name, Vec::new()))
            .collect();
        lists.push((YOURS, Vec::new()));
        for b in &self.bindings {
            let keys = parse_keys(&b.key).unwrap_or_default();
            let at = held.iter().position(|built| built.contains(&keys));
            if let Some((_, list)) = lists.get_mut(at.unwrap_or(held.len())) {
                list.push(b);
            }
        }
        lists.retain(|(_, list)| !list.is_empty());
        lists
    }

    /// The first key bound to `command` with no arguments, as written in the file.
    pub fn key_for(&self, command: &str) -> Option<&str> {
        self.key_for_args(command, &toml::Table::new())
    }

    /// The first key bound to `command` with exactly `args` (its [`Binding::plain_args`]), as
    /// written in the file. A slider's arguments (one marked `"drag"`) are also the key's that
    /// turns the same argument off and on (marked `"flip"`): L is the Smoothing slider's. So
    /// are a switch's as a tap runs them, true or false in place of `"flip"`, as the palette
    /// lists them: Ctrl+H is Show mask's. A brush's pick ([`picked`]) takes the key that
    /// picks that brush ([`Keymap::brush_key`]), whichever of the two commands runs it.
    pub fn key_for_args(&self, command: &str, args: &toml::Table) -> Option<&str> {
        if let Some(brush) = picked(command, args) {
            return self.brush_key(brush);
        }
        let flips = flipped(args);
        (self.bindings.iter())
            .find(|b| b.command == command && [args, &flips].contains(&&b.plain_args()))
            .map(|b| b.key.as_str())
    }
}

/// `args` with an argument marked `"drag"`, or given as true or false, marked `"flip"` instead,
/// looking inside tables.
pub(crate) fn flipped(args: &toml::Table) -> toml::Table {
    let flip = |v: &toml::Value| match v {
        toml::Value::String(s) if s == "drag" => "flip".into(),
        toml::Value::Boolean(_) => "flip".into(),
        toml::Value::Table(t) => toml::Value::Table(flipped(t)),
        v => v.clone(),
    };
    args.iter().map(|(k, v)| (k.clone(), flip(v))).collect()
}

/// The `ui.keymap` step that binds `keys` to run `runs`, its arguments with it when it has any:
/// what a Ctrl+Alt press on a control (`keys/catch.rs`) and Bind a key… (`palette/bind.rs`)
/// send.
pub(crate) fn binding(keys: &str, runs: &gs_protocol::Step) -> gs_protocol::Step {
    let mut args = serde_json::json!({ "bind": keys, "command": runs.command });
    if runs.args.as_object().is_some_and(|a| !a.is_empty()) {
        args["args"] = runs.args.clone();
    }
    crate::record::with_args("ui.keymap", args)
}

/// A command and its arguments on one line: `brush.set size_step = -1`.
pub(crate) fn inline(command: &str, args: &toml::Table) -> String {
    let args: Vec<String> = args.iter().map(|(k, v)| format!("{k} = {v}")).collect();
    format!("{command} {}", args.join(", "))
        .trim_end()
        .to_string()
}
