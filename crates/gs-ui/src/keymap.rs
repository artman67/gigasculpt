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

#![expect(
    clippy::result_large_err,
    reason = "ui.keymap answers with the engine's wire error, as every command does (gs-engine's lib.rs)"
)]

use std::path::Path;
use std::sync::LazyLock;

use egui::{Key, Modifiers};
use gs_engine::{KeyBinding, KeymapInfo, Slots, UiKeymap, Unavailable, errors};
use gs_protocol::CommandError;
use serde::{Deserialize, Serialize};

use crate::direction::parse;
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

/// The built-in keys whose other arguments their command's title does not say, each with a
/// title of its own that the keys sheet reads (decision 0062, row 818); the palette keeps its
/// command's.
const TITLES: [(&str, &str); 7] = [
    ("Shift+Space", "Quick menu"),
    ("F6", "Materials"),
    ("Ctrl+L", "Show layout…"),
    ("Ctrl+Shift+Alt+I", "Save layout…"),
    ("[", "Size down"),
    ("]", "Size up"),
    ("C", "Pick color"),
];

/// Every binding: the built-in ones with the user's changes over them.
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    pub bindings: Vec<Binding>,
    /// The user's changes, as the keymap file lists them.
    pub changes: Vec<Binding>,
}

/// The keymap file: the user's changes to the built-in keys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Changes {
    /// 2 since decision 0025; left out by the old rule, which wrote every default key.
    #[serde(default)]
    version: u32,
    #[serde(rename = "bind", default)]
    bindings: Vec<Binding>,
}

/// The file's version since the built-in keys always apply.
const VERSION: u32 = 2;

/// Where a keymap file written by the old rule is moved, beside the new one.
const BEFORE: &str = "keymap-before-0025.toml";

/// A keymap file with no changes yet.
const EMPTY: &str =
    "# GigaSculpt keymap: your changes to the built-in keys, which always apply. Each
# binding runs one command; keys look like Ctrl+Shift+Z, and keys separated by spaces are a
# sequence. A binding for a key that has a built-in one replaces it; an empty command unbinds
# the key. For example, the palette on Tab instead of Space:
#
# [[bind]]
# key = \"Tab\"
# command = \"ui.palette\"
#
# [[bind]]
# key = \"Space\"
# command = \"\"

version = 2
";

/// One built-in key: the key, the command it runs, and the command's arguments written as TOML.
type Row = (&'static str, &'static str, &'static str);

/// The built-in keys by context, in the order the keys sheet lists them, each context a table of
/// the catalog: General (`interface.default-bindings`), View (`-view`), Sculpt (`-sculpt`), Mask
/// (`-mask`) and Geometry (`-geometry`). Every key works in the window outside a text field. A
/// catalog row whose command does not exist yet is reserved, not bound (the catalog's notes
/// list them).
const TABLE: [(&str, &[Row]); 5] = [
    ("General", GENERAL),
    ("View", VIEW),
    ("Sculpt", SCULPT),
    ("Mask", MASK),
    ("Geometry", GEOMETRY),
];

/// [`TABLE`] as bindings.
static BUILT_IN: LazyLock<[(&str, Vec<Binding>); 5]> = LazyLock::new(|| {
    TABLE.map(|(context, rows)| {
        let bindings = rows
            .iter()
            .map(|&(key, command, args)| bind(key, command, args));
        (context, bindings.collect())
    })
});

/// The context the keys sheet lists keys under that only the user's changes bind.
const YOURS: &str = "Your";

impl Default for Keymap {
    /// The built-in keys, every context's (see [`BUILT_IN`]).
    fn default() -> Self {
        Self {
            bindings: BUILT_IN.iter().flat_map(|(_, b)| b.clone()).collect(),
            changes: Vec::new(),
        }
    }
}

/// General keys (`interface.default-bindings`): the palette on Space (the left thumb; the
/// catalog puts the quick menu there, decision 0025 the palette, provisionally) and the quick
/// menu on Shift+Space (Blender's popup key); undo, redo and save where every program has them;
/// Ctrl+O opens a scene and Ctrl+Shift+T saves the active part as a Part file, Ctrl+N starts a
/// new scene and Ctrl+Alt+R reverts to the saved one, each through the palette's form as its
/// button does, so a key alone never drops work; Ctrl+Alt+S saves the next numbered file
/// (Blender's incremental save) and 9 a quick save; Tab hides and shows every panel and Comma
/// the Library; F2 held opens the brush wheel at the pen (let go to pick); F9 shows the adjust panel again for the newest step; Ctrl+L shows a layout by name
/// and Ctrl+Shift+Alt+I saves the one shown as a layout file; Ctrl+1 to 3 show the
/// arrangements saved as `arrangement-1` to 3, and Ctrl+Shift+1 to 3 save the one shown there
/// (provisional, decision 0025). A key through the form runs `ui.palette` with `form` naming the
/// command and the rest of its arguments after it.
const GENERAL: &[Row] = &[
    ("Space", "ui.palette", ""),
    ("Shift+Space", "ui.quick", "open = true"),
    ("Ctrl+Z", "history.undo", ""),
    ("Ctrl+Shift+Z", "history.redo", ""),
    ("Ctrl+Y", "history.redo", ""),
    ("Ctrl+S", "file.save", ""),
    (
        "Ctrl+O",
        "ui.palette",
        "form = \"file.open\"\nask = [\"path\"]",
    ),
    (
        "Ctrl+Shift+T",
        "ui.palette",
        "form = \"file.save\"\nargs = { parts = [\"@active\"] }\nask = [\"path\"]",
    ),
    (
        "Ctrl+N",
        "ui.palette",
        "form = \"file.new\"\nask = [\"discard_changes\"]",
    ),
    ("Ctrl+Alt+R", "ui.palette", "form = \"file.revert\""),
    ("Ctrl+Alt+S", "file.save_next", ""),
    ("9", "file.quick_save", ""),
    ("Tab", "ui.layout", "chrome = \"flip\""),
    ("Comma", "ui.library", "open = \"flip\""),
    ("F2", "ui.wheel", "open = \"held\""),
    // F6 opens the Library's Materials tab, only off the 3D view (decision 0025, section 5).
    ("F6", "ui.library", "tab = \"matcaps\"\nopen = \"off-view\""),
    // F9 shows the adjust panel again (Blender's key; decision 0060, section 5, as amended).
    ("F9", "ui.adjust", "open = true"),
    (
        "Ctrl+L",
        "ui.palette",
        "form = \"ui.layout\"\nask = [\"name\"]",
    ),
    (
        "Ctrl+Shift+Alt+I",
        "ui.palette",
        "form = \"ui.layout\"\nask = [\"save\"]",
    ),
    ("Ctrl+1", "ui.layout", "name = \"arrangement-1\""),
    ("Ctrl+Shift+1", "ui.layout", "save = \"arrangement-1\""),
    ("Ctrl+2", "ui.layout", "name = \"arrangement-2\""),
    ("Ctrl+Shift+2", "ui.layout", "save = \"arrangement-2\""),
    ("Ctrl+3", "ui.layout", "name = \"arrangement-3\""),
    ("Ctrl+Shift+3", "ui.layout", "save = \"arrangement-3\""),
];

/// View keys (`interface.default-bindings-view`, decisions 0017 and 0025), most acting on the
/// view under the pointer, a Reference View's picture or else the 3D view: F frames every part,
/// Alt+F the mask, Period the last edit (Blender's Numpad period in sculpt mode), Shift+F
/// switches the wireframe, P switches perspective, V then X, Y or Z turns to an axis view
/// (again: the opposite side), V I the iso view, V H levels the view, V L locks the camera, and
/// Alt+Z and Shift+Alt+Z step back and forward through the camera history. Shift+P shows and
/// hides the floor grid and X turns X symmetry on and off. `view =
/// "pointed"` is [`POINTED`]: the view under the pointer.
const VIEW: &[Row] = &[
    ("F", "view.frame", "view = \"pointed\""),
    (
        "Alt+F",
        "view.frame",
        "target = \"mask\"\nview = \"pointed\"",
    ),
    (
        "Period",
        "view.frame",
        "target = \"last-edit\"\nview = \"pointed\"",
    ),
    (
        "Shift+F",
        "view.look",
        "flip_wireframe = true\nview = \"pointed\"",
    ),
    (
        "P",
        "view.lens",
        "projection = \"toggle\"\nview = \"pointed\"",
    ),
    ("Shift+P", "view.display", "floor = \"flip\""),
    ("X", "brush.set", "flip_mirror = \"x\""),
    (
        "V X",
        "view.align",
        "to = \"left\"\ntoggle = true\nview = \"pointed\"",
    ),
    (
        "V Y",
        "view.align",
        "to = \"top\"\ntoggle = true\nview = \"pointed\"",
    ),
    (
        "V Z",
        "view.align",
        "to = \"front\"\ntoggle = true\nview = \"pointed\"",
    ),
    ("V I", "view.align", "to = \"iso\"\nview = \"pointed\""),
    ("V H", "view.level", "view = \"pointed\""),
    ("V L", "view.lock", "view = \"pointed\""),
    ("Alt+Z", "view.undo", "view = \"pointed\""),
    ("Shift+Alt+Z", "view.redo", "view = \"pointed\""),
];

/// Sculpting keys for the left hand (`interface.default-bindings-sculpt`): B and two letters
/// pick a brush (decision 0017 as amended, item 11), every shipped brush its own, its letters
/// from its name where they are free; S, U and O held while moving set size,
/// strength and Falloff; [ and ] step the size; L turns stroke smoothing off and on; the
/// backtick goes back to the previous brush; Shift+X swaps the Main and Secondary colors (decision 0039, section 11: X is Mirror
/// X and V starts the view sequences), and C picks the color under the pen into the edited
/// one (`pieces/picker.rs`).
const SCULPT: &[Row] = &[
    ("B S T", "brush.set", "brush = \"Draw\""),
    ("B C B", "brush.set", "brush = \"Clay Build\""),
    ("B S S", "brush.set", "brush = \"Smooth Strong\""),
    ("B M E", "brush.set", "brush = \"Move Elastic\""),
    ("B M P", "brush.set", "brush = \"Smooth Polish\""),
    ("B C C", "brush.set", "brush = \"Clip Curve\""),
    ("B C L", "brush.set", "brush = \"Clay Line\""),
    ("B M V", "brush.set", "brush = \"Move\""),
    ("B S M", "brush.set", "brush = \"Smooth\""),
    ("B I N", "brush.set", "brush = \"Inflate\""),
    ("B P I", "brush.set", "brush = \"Pinch\""),
    ("B D S", "brush.set", "brush = \"Sharp Crease\""),
    ("B S H", "brush.set", "brush = \"Pull Out\""),
    ("B T D", "brush.set", "brush = \"Trim\""),
    ("B H P", "brush.set", "brush = \"Flat Polish\""),
    ("B C A", "brush.set", "brush = \"Clay\""),
    ("B M C", "brush.set", "brush = \"Move Connected\""),
    ("B P N", "brush.set", "brush = \"Pull Out Along Normal\""),
    ("B M K", "brush.set", "brush = \"Mask Pen\""),
    ("B S R", "brush.set", "brush = \"Select Rect\""),
    ("B U D", "brush.set", "brush = \"UV Density\""),
    ("B U H", "brush.set", "brush = \"UV Hint\""),
    ("B P A", "brush.set", "brush = \"Paint\""),
    ("B P B", "brush.set", "brush = \"Paint Blend\""),
    ("S", "brush.set", "size_px = \"drag\""),
    ("U", "brush.set", "strength = \"drag\""),
    ("O", "brush.set", "falloff = \"drag\""),
    ("[", "brush.set", "size_step = -1"),
    ("]", "brush.set", "size_step = 1"),
    (
        "L",
        "brush.set",
        "settings.stroke.smoothing.radius = \"flip\"",
    ),
    ("Backtick", "brush.set", "previous = true"),
    ("Shift+X", "color.set", "swap = true"),
    ("C", "color.set", "pick = \"pointer\""),
];

/// Mask, visibility and face group keys (`interface.default-bindings-mask`, decision 0017 as
/// amended, item 11), on the active part: Ctrl+H shows and hides the mask's darkening,
/// Ctrl+I inverts the mask, Ctrl+A masks everything and Alt+M
/// clears it, Invert and Clear taking hidden points too (`R9`); Ctrl+W groups the masked faces
/// and clears the mask; Ctrl+Shift+X grows the visible area, Ctrl+Shift+S shrinks it, and
/// Ctrl+Shift+A and Ctrl+Shift+Q grow it to whole shells and whole groups.
const MASK: &[Row] = &[
    ("Ctrl+H", "view.display", "mask = \"flip\""),
    ("Ctrl+I", "mask.invert", "scope = \"all\""),
    ("Ctrl+A", "mask.set", "value = 1"),
    ("Alt+M", "mask.set", "value = 0\nscope = \"all\""),
    (
        "Ctrl+W",
        "group.make",
        "maker = { by = \"masked\", clear_mask = true }",
    ),
    ("Ctrl+Shift+X", "visibility.grow", ""),
    ("Ctrl+Shift+S", "visibility.shrink", ""),
    ("Ctrl+Shift+A", "visibility.grow", "to = \"shells\""),
    ("Ctrl+Shift+Q", "visibility.grow", "to = \"groups\""),
];

/// Geometry and part keys (`interface.default-bindings-geometry`, decision 0017 as amended,
/// item 11), on the active part: Ctrl+D subdivides, D and Shift+D step a detail level up and
/// down, Ctrl+R voxel remeshes, Backslash turns Live Detail on and off, and Ctrl+Shift+D
/// duplicates the part.
const GEOMETRY: &[Row] = &[
    ("Ctrl+D", "level.subdivide", ""),
    ("D", "level.step", "by = 1"),
    ("Shift+D", "level.step", "by = -1"),
    ("Ctrl+R", "mesh.voxel_remesh", ""),
    ("Backslash", "live_detail.set", "toggle = true"),
    ("Ctrl+Shift+D", "part.duplicate", ""),
];

/// `key` running `command` with `args`, written as TOML.
fn bind(key: &str, command: &str, args: &str) -> Binding {
    Binding {
        key: key.into(),
        command: command.into(),
        args: toml::from_str(args).unwrap_or_default(),
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

impl Keymap {
    /// Reads the keymap file and applies its changes to the built-in keys. A missing file is
    /// written with no changes, for the user to edit. A file written by the old rule (no
    /// `version = 2`) is moved aside to `keymap-before-0025.toml` and an empty one takes its
    /// place; the second value says so, once. A change naming a command the program does not
    /// have is refused alone and the second value names it (R8, decision 0025). A file that
    /// does not read is left alone and reported.
    pub fn load(path: &Path) -> Result<(Self, Option<String>), String> {
        let name = path.display().to_string();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                write_file(path, EMPTY)?;
                return Ok((Self::default(), None));
            }
            Err(e) => return Err(format!("{name}: {e}")),
        };
        let changes: Changes = parse(&name, &text)?;
        let file = path
            .file_name()
            .map_or(name.clone(), |f| f.display().to_string());
        let (known, refused) =
            known(changes.bindings, &file).map_err(|e| format!("{name}: {e}"))?;
        let keymap = Self::default().with(known);
        let keymap = keymap.map_err(|e| format!("{name}: {e}"))?;
        match changes.version {
            VERSION => Ok((keymap, (!refused.is_empty()).then(|| refused.join("; ")))),
            0 => Ok((Self::default(), Some(move_aside(path)?))),
            v => Err(format!(
                "{name}: version {v} is unknown; this version reads {VERSION}"
            )),
        }
    }

    /// These bindings with `changes` over them: each takes its key from every binding it
    /// clashes with (the same key, or a sequence one of the two starts), and one with an empty
    /// command only removes the binding for its key.
    fn with(mut self, changes: Vec<Binding>) -> Result<Self, String> {
        for change in &changes {
            let keys = parse_keys(&change.key)?;
            let unbinds = change.command.is_empty();
            self.bindings.retain(|b| !same_keys(b, &keys, !unbinds));
            if !unbinds {
                self.bindings.push(change.clone());
            }
        }
        self.changes.extend(changes);
        Ok(self)
    }

    /// Binds a key or puts keys back as `change` asks (`ui.keymap`, decision 0025 section 5),
    /// writing the changes alone to `file` when given. Nothing changes unless all of it does.
    pub fn change(
        &mut self,
        change: &UiKeymap,
        file: Option<&Path>,
    ) -> Result<KeymapInfo, CommandError> {
        let mut changes = self.changes.clone();
        if change.restore_defaults {
            changes.clear();
        } else if let Some(key) = &change.restore {
            let keys = keys_arg("/restore", key)?;
            let at = changes.iter().position(|c| same_keys(c, &keys, false));
            let Some(at) = at else {
                let changed = changes.iter().map(|c| c.key.as_str());
                return Err(errors::not_found("changed key", key, changed));
            };
            changes.remove(at);
        } else if let Some(key) = &change.bind {
            self.bind(&mut changes, key, change)?;
        }
        let keymap = Self::default().with(changes);
        let keymap = keymap.map_err(|e| errors::internal(&e))?;
        if let Some(file) = file {
            write_file(file, &keymap.file_text()?).map_err(|e| errors::internal(&e))?;
        }
        *self = keymap;
        Ok(self.info())
    }

    /// Puts the binding `change` asks for `key` among `changes`, in place of the changes it
    /// clashes with; one the built-in keys hold already is no change. A key another binding
    /// holds is refused naming each holder, asking for `replace`.
    fn bind(
        &self,
        changes: &mut Vec<Binding>,
        key: &str,
        change: &UiKeymap,
    ) -> Result<(), CommandError> {
        let keys = keys_arg("/bind", key)?;
        let args = serde_json::from_value(serde_json::Value::Object(change.args.clone()));
        let new = Binding {
            key: written(key).map_err(|e| errors::args_invalid("/bind", e))?,
            command: change.command.clone().unwrap_or_default(),
            args: args.map_err(|e| errors::args_invalid("/args", e.to_string()))?,
        };
        let unbinds = new.command.is_empty();
        let holders: Vec<String> = (self.bindings.iter())
            .filter(|b| **b != new && same_keys(b, &keys, true))
            .map(holder)
            .collect();
        if !unbinds && !holders.is_empty() && !change.replace {
            let reason = format!("{} is taken: {}", new.key, holders.join("; "));
            let fix = "give replace = true to move the key, dropping what holds it";
            return Err(errors::unavailable(
                Unavailable::new(reason).fix(fix).asks("replace"),
            ));
        }
        changes.retain(|c| !same_keys(c, &keys, !unbinds));
        let built_in = Self::default().bindings;
        let mut held = built_in.iter().filter(|b| same_keys(b, &keys, false));
        let no_change = if unbinds {
            held.next().is_none()
        } else {
            held.any(|b| b.command == new.command && b.args == new.args)
        };
        if !no_change {
            changes.push(new);
        }
        Ok(())
    }

    /// The keymap file for these changes: the header of a new one, then each change.
    fn file_text(&self) -> Result<String, CommandError> {
        #[derive(Serialize)]
        struct Written<'a> {
            bind: &'a [Binding],
        }
        if self.changes.is_empty() {
            return Ok(EMPTY.into());
        }
        let written = toml::to_string(&Written {
            bind: &self.changes,
        });
        Ok(format!(
            "{EMPTY}\n{}",
            written.map_err(|e| errors::internal(&e.to_string()))?
        ))
    }

    /// The changes and every binding, as `ui.keymap` answers.
    fn info(&self) -> KeymapInfo {
        let plain = |b: &Binding| KeyBinding {
            key: b.key.clone(),
            command: b.command.clone(),
            args: match serde_json::to_value(&b.args) {
                Ok(serde_json::Value::Object(args)) => args,
                _ => serde_json::Map::new(),
            },
        };
        KeymapInfo {
            changes: self.changes.iter().map(plain).collect(),
            bindings: self.bindings.iter().map(plain).collect(),
        }
    }

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

    /// The key that picks `brush`, whatever its button runs (`brush.set`, or `brush.slot` for a
    /// Smooth brush): the newest binding of either to that brush alone, so a key the user bound
    /// to it shows over its built-in one, and none once every key for it is removed.
    fn brush_key(&self, brush: &str) -> Option<&str> {
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

/// The changes in `file` whose command the program has (an empty one unbinds), and a note for
/// each of the rest, refused alone so its key keeps what it had (R8, decision 0025). Every key
/// is read first, so a key that does not read still refuses the whole file.
fn known(changes: Vec<Binding>, file: &str) -> Result<(Vec<Binding>, Vec<String>), String> {
    let registry = gs_engine::registry();
    let names = || registry.iter().map(|spec| spec.name);
    let built_in = Keymap::default();
    let (mut known, mut notes) = (Vec::new(), Vec::new());
    for change in changes {
        let keys = parse_keys(&change.key)?;
        if change.command.is_empty() || names().any(|n| n == change.command) {
            known.push(change);
            continue;
        }
        let near = gs_engine::errors::did_you_mean(&change.command, names());
        let near = (near.first()).map_or(String::new(), |n| format!(" (did you mean {n}?)"));
        let kept = match built_in.sequence(&keys) {
            Pressed::Binding(_) => "keeps its built-in binding",
            Pressed::Prefix | Pressed::Nothing => "stays unbound",
        };
        let (key, command) = (&change.key, &change.command);
        notes.push(format!(
            "{file}: {key} runs {command}, which is not a command{near}; {key} {kept}"
        ));
    }
    Ok((known, notes))
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

/// Whether `b`'s key is `keys`, or with `or_starts` one of the two starts the other (B against
/// B S T): a key that starts a sequence can never run alone.
fn same_keys(b: &Binding, keys: &[Chord], or_starts: bool) -> bool {
    parse_keys(&b.key)
        .is_ok_and(|k| k == keys || (or_starts && (k.starts_with(keys) || keys.starts_with(&k))))
}

/// The key `ui.keymap` names at `path`, read.
fn keys_arg(path: &str, key: &str) -> Result<Vec<Chord>, CommandError> {
    parse_keys(key).map_err(|e| errors::args_invalid(path, e))
}

/// A binding that holds a key, as a refusal names it: "Ctrl+D runs Subdivide
/// (level.subdivide) in the window", the window being the one context today.
fn holder(b: &Binding) -> String {
    let specs = gs_engine::registry();
    let title = (specs.iter().find(|s| s.name == b.command)).map_or("", |s| s.title);
    let runs = inline(&b.command, &b.plain_args());
    format!("{} runs {title} ({runs}) in the window", b.key)
}

/// A command and its arguments on one line: `brush.set size_step = -1`.
pub(crate) fn inline(command: &str, args: &toml::Table) -> String {
    let args: Vec<String> = args.iter().map(|(k, v)| format!("{k} = {v}")).collect();
    format!("{command} {}", args.join(", "))
        .trim_end()
        .to_string()
}

/// A key or a sequence as the keymap file writes it: `shift + ctrl+z` is `Ctrl+Shift+Z`.
fn written(text: &str) -> Result<String, String> {
    let keys = parse_keys(text)?;
    Ok((keys.iter().map(ToString::to_string))
        .collect::<Vec<_>>()
        .join(" "))
}

/// The chords of a key or a sequence of keys separated by spaces (spaces around a `+` join).
fn parse_keys(text: &str) -> Result<Vec<Chord>, String> {
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

/// Moves a keymap file written by the old rule aside and puts an empty one in its place.
/// Returns what the status line says.
fn move_aside(path: &Path) -> Result<String, String> {
    let before = path.with_file_name(BEFORE);
    if before.exists() {
        let name = before.display();
        return Err(format!(
            "{} is an old keymap and {name} is taken: move one away",
            path.display()
        ));
    }
    std::fs::rename(path, &before).map_err(|e| format!("{}: {e}", path.display()))?;
    write_file(path, EMPTY)?;
    Ok(format!(
        "the built-in keys apply again; your old keymap is in {BEFORE}, and keymap.toml now holds only your changes"
    ))
}

/// Writes `text` to `path`, making its folder first.
pub(crate) fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}
