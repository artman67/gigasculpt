//! What a binding is: a key or wheel notch with its modifiers ([`Chord`]), a binding of keys to
//! a command ([`Binding`]), and how keys are read and written.

use egui::{Key, Modifiers};
use serde::{Deserialize, Serialize};

use super::built_in::{BUILT_IN, TITLES};
use crate::reference::POINTED;

/// What a chord presses: a key, or a notch of the wheel turned up or down (a tablet's touch ring
/// sends wheel turns too).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Key(Key),
    WheelUp,
    WheelDown,
}

impl Press {
    /// The key pressed, unless this is a wheel notch.
    pub fn as_key(self) -> Option<Key> {
        match self {
            Self::Key(key) => Some(key),
            Self::WheelUp | Self::WheelDown => None,
        }
    }

    /// Its name as the keymap file writes it: egui's name for a key, `WheelUp` or `WheelDown`.
    fn name(self) -> &'static str {
        match self {
            Self::Key(key) => key.name(),
            Self::WheelUp => "WheelUp",
            Self::WheelDown => "WheelDown",
        }
    }

    /// Reads a name the keymap file writes.
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "WheelUp" => Some(Self::WheelUp),
            "WheelDown" => Some(Self::WheelDown),
            name => Key::from_name(name).map(Self::Key),
        }
    }
}

impl From<Key> for Press {
    fn from(key: Key) -> Self {
        Self::Key(key)
    }
}

/// A key or a wheel notch with the modifiers held.
#[expect(
    clippy::struct_excessive_bools,
    reason = "four independent modifier keys"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: Press,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
}

impl Chord {
    /// A key pressed, or the wheel turned, with `modifiers`. On every platform `ctrl` means the
    /// Control key.
    pub fn pressed(key: impl Into<Press>, modifiers: Modifiers) -> Self {
        Self {
            key: key.into(),
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            logo: modifiers.mac_cmd,
        }
    }

    /// Reads `Ctrl+Shift+Z`, `Alt+WheelUp` and the like.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let name = parts.pop().unwrap_or_default();
        let key =
            Press::from_name(name).ok_or_else(|| format!("unknown key '{name}' in '{text}'"))?;
        let mut chord = Self::pressed(key, Modifiers::NONE);
        for modifier in parts {
            let flag = match modifier.to_ascii_lowercase().as_str() {
                "ctrl" => &mut chord.ctrl,
                "shift" => &mut chord.shift,
                "alt" => &mut chord.alt,
                "super" => &mut chord.logo,
                other => return Err(format!("unknown modifier '{other}' in '{text}'")),
            };
            *flag = true;
        }
        Ok(chord)
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let held = [
            (self.ctrl, "Ctrl+"),
            (self.shift, "Shift+"),
            (self.alt, "Alt+"),
            (self.logo, "Super+"),
        ];
        for (on, name) in held {
            if on {
                f.write_str(name)?;
            }
        }
        f.write_str(self.key.name())
    }
}

/// One binding: a key and the command it runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub key: String,
    pub command: String,
    /// The command's arguments.
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    pub args: toml::Table,
}

impl Binding {
    /// Its arguments without `view = "pointed"`: a key that acts on the view under the pointer
    /// does over the 3D view what a button with the rest of its arguments does.
    pub fn plain_args(&self) -> toml::Table {
        let mut args = self.args.clone();
        if args.get("view").and_then(toml::Value::as_str) == Some(POINTED) {
            args.remove("view");
        }
        args
    }

    /// Its own title while it is a built-in key that has one ([`TITLES`]), for the keys sheet.
    pub fn title(&self) -> Option<&'static str> {
        let (key, title) = TITLES.iter().find(|(key, _)| *key == self.key)?;
        let built = BUILT_IN.iter().flat_map(|(_, b)| b).find(|b| b.key == *key);
        (built == Some(self)).then_some(*title)
    }
}

/// What a sequence of key presses means.
#[derive(Debug, PartialEq)]
pub enum Pressed<'a> {
    /// It completes this binding.
    Binding(&'a Binding),
    /// It starts at least one longer sequence.
    Prefix,
    /// Nothing.
    Nothing,
}

/// Whether `b`'s key is `keys`, or with `or_starts` one of the two starts the other (B against
/// B S T): a key that starts a sequence can never run alone.
pub(super) fn same_keys(b: &Binding, keys: &[Chord], or_starts: bool) -> bool {
    parse_keys(&b.key)
        .is_ok_and(|k| k == keys || (or_starts && (k.starts_with(keys) || keys.starts_with(&k))))
}

/// A key or a sequence as the keymap file writes it: `shift + ctrl+z` is `Ctrl+Shift+Z`.
pub(super) fn written(text: &str) -> Result<String, String> {
    let keys = parse_keys(text)?;
    Ok((keys.iter().map(ToString::to_string))
        .collect::<Vec<_>>()
        .join(" "))
}

/// The chords of a key or a sequence of keys separated by spaces (spaces around a `+` join).
pub(super) fn parse_keys(text: &str) -> Result<Vec<Chord>, String> {
    let joined: Vec<&str> = text.split('+').map(str::trim).collect();
    let joined = joined.join("+");
    let keys: Vec<Chord> = joined
        .split_whitespace()
        .map(Chord::parse)
        .collect::<Result<_, _>>()?;
    if keys.is_empty() {
        return Err(format!("no key in '{text}'"));
    }
    Ok(keys)
}
