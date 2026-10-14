//! The keymap's own file and command: reading the user's changes over the built-in keys, and
//! `ui.keymap` binding a key or putting keys back, writing the changes alone.

#![expect(
    clippy::result_large_err,
    reason = "ui.keymap answers with the engine's wire error, as every command does (gs-engine's lib.rs)"
)]

use std::path::Path;

use gs_engine::{KeyBinding, KeymapInfo, UiKeymap, Unavailable, errors};
use gs_protocol::CommandError;
use serde::{Deserialize, Serialize};

use super::binding::{parse_keys, same_keys, written};
use super::{Binding, Chord, Keymap, Pressed};
use crate::direction::parse;

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

/// The key `ui.keymap` names at `path`, read.
fn keys_arg(path: &str, key: &str) -> Result<Vec<Chord>, CommandError> {
    parse_keys(key).map_err(|e| errors::args_invalid(path, e))
}

/// A binding that holds a key, as a refusal and the Replace question name it, by title, not
/// id (review 0009): "Ctrl+D runs Subdivide in the window", a brush key by its brush ("B S T
/// runs Draw"), a built-in key with a title of its own by that; the window is the one context
/// today. The keys sheet names a binding much the same way, a control's label first
/// (`pieces/key_sheet.rs` `action`), which the keymap cannot reach.
fn holder(b: &Binding) -> String {
    let specs = gs_engine::registry();
    let title =
        (specs.iter().find(|s| s.name == b.command)).map_or(b.command.as_str(), |s| s.title);
    let brush = (b.args.len() == 1)
        .then(|| b.args.get("brush")?.as_str())
        .flatten();
    let does = b.title().or(brush).unwrap_or(title);
    format!("{} runs {does} in the window", b.key)
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
