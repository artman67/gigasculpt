//! The brush keys: which key picks a brush, whichever command its button runs, and the held
//! keys whose slot holds it.

use gs_engine::Slots;

use super::Keymap;

impl Keymap {
    /// The key that picks `brush`, whatever its button runs (`brush.set`, or `brush.slot` for a
    /// Smooth brush): the newest binding of either to that brush alone, so a key the user bound
    /// to it shows over its built-in one, and none once every key for it is removed.
    pub(super) fn brush_key(&self, brush: &str) -> Option<&str> {
        (self.bindings.iter().rev())
            .find(|b| picked(&b.command, &b.plain_args()) == Some(brush))
            .map(|b| b.key.as_str())
    }

    /// What picks `brush`: its key ([`Keymap::brush_key`]), and the held keys whose slot in
    /// `slots` holds it, which stroke with it ([`crate::held::holding`]).
    pub fn brush_keys(&self, brush: &str, slots: &Slots) -> BrushKeys {
        BrushKeys {
            key: self.brush_key(brush).map(Into::into),
            held: crate::held::holding(brush, slots),
        }
    }
}

/// The commands that pick a brush by its name: its key runs `brush.set`, a Smooth brush's
/// button `brush.slot` (`Directions::brush_controls`).
const PICKS: [&str; 2] = ["brush.set", "brush.slot"];

/// The brush `command` with `args` picks, if it picks one by name: one of [`PICKS`] given
/// `brush` and nothing else.
pub(crate) fn picked<'a>(command: &str, args: &'a toml::Table) -> Option<&'a str> {
    let brush = args.get("brush").and_then(toml::Value::as_str)?;
    (PICKS.contains(&command) && args.len() == 1).then_some(brush)
}

/// What picks a brush ([`Keymap::brush_keys`]): its key, and the held keys whose slot holds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrushKeys {
    /// The key bound to it, as the keymap writes it: `B S M`.
    pub key: Option<String>,
    /// The modifier keys that stroke with it while held, its slot's: `Shift`, `Ctrl+Shift`.
    pub held: Option<String>,
}

/// What a brush nothing picks says.
pub const NO_KEY: &str = "No key";

impl BrushKeys {
    /// In one line: `B S M or Shift held`, `B S T`, `Ctrl held`, or [`NO_KEY`].
    pub fn text(&self) -> String {
        match (&self.key, &self.held) {
            (Some(key), Some(held)) => format!("{key} or {held} held"),
            (Some(key), None) => key.clone(),
            (None, Some(held)) => format!("{held} held"),
            (None, None) => NO_KEY.into(),
        }
    }
}
