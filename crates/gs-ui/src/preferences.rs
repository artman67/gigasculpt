//! The preferences file, `preferences.toml` in the window's config folder (decision 0052,
//! section 3): `version = 1`, then each setting under the part of its `session.state` name
//! before the first dot (`pen.lazy_pressure` is `lazy_pressure` in `[pen]`). The whole file is
//! kept as read, so a key this version does not know is written back unchanged. A file from a
//! newer version, or one that does not read as TOML, is read as far as it reads and never
//! written. Which settings go in it, and when, is the shell's (`shell/prefs.rs`).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::keymap::write_file;

/// The version this program writes and reads.
const VERSION: i64 = 1;

/// The file's first line.
const HEADER: &str =
    "# GigaSculpt preferences, saved automatically. Keys this version does not know are kept.";

/// The preferences, as read and as changed since.
#[derive(Debug, Default)]
pub struct Preferences {
    /// Everything the file holds but its version.
    table: toml::Table,
    /// Where it is saved; `None`: nowhere.
    path: Option<PathBuf>,
    /// Why it is never written (a newer version, a file that does not read).
    frozen: Option<String>,
    /// The table differs from the file.
    changed: bool,
    /// The window has started from it: its values were run (decision 0052, section 4).
    pub started: bool,
    /// A call other than the pen's samples was heard since the settings were last kept: one
    /// may have changed.
    pub heard: bool,
}

impl Preferences {
    /// Preferences kept in no file.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// The preferences in `path` (none yet when there is no file), and what is wrong with the
    /// file, for the status line.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        let mut prefs = Self {
            path: Some(path.to_path_buf()),
            ..Self::default()
        };
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (prefs, None),
            Err(e) => return prefs.frozen(&format!("{}: {e}", path.display())),
        };
        let mut table = match text.parse::<toml::Table>() {
            Ok(table) => table,
            Err(e) => return prefs.frozen(&format!("{} does not read: {e}", path.display())),
        };
        let version = table.remove("version").and_then(|v| v.as_integer());
        prefs.table = table;
        match version {
            Some(v) if v > VERSION => prefs.frozen(&format!(
                "{} is version {v}, newer than this GigaSculpt writes ({VERSION})",
                path.display()
            )),
            _ => (prefs, None),
        }
    }

    /// Never written, for `why`.
    fn frozen(mut self, why: &str) -> (Self, Option<String>) {
        let said = format!("{why}: its settings are read, and changes last this session");
        self.frozen = Some(said.clone());
        (self, Some(said))
    }

    /// The value kept for `name` (`pen.lazy_pressure`), if the file has it.
    pub fn get(&self, name: &str) -> Option<Value> {
        let (section, key) = name.split_once('.')?;
        let value = self.table.get(section)?.as_table()?.get(key)?;
        serde_json::to_value(value).ok()
    }

    /// Keeps `value` for `name`; a null, or a name without a section, is not kept.
    pub fn put(&mut self, name: &str, value: &Value) {
        let (Some((section, key)), Some(value)) = (name.split_once('.'), written(value)) else {
            return;
        };
        let entry = self
            .table
            .entry(section)
            .or_insert_with(|| toml::Table::new().into());
        let Some(table) = entry.as_table_mut() else {
            return;
        };
        if table.get(key) != Some(&value) {
            table.insert(key.to_string(), value);
            self.changed = true;
        }
    }

    /// Keeps each of `moved` (an older `layout.toml`'s settings, by name) the file does not hold
    /// yet, and writes the file (decision 0052 as amended by step Prefs-c).
    pub fn hand_over(&mut self, moved: &[(String, Value)]) -> Result<(), String> {
        for (name, value) in moved {
            if self.get(name).is_none() {
                self.put(name, value);
            }
        }
        self.save_if_changed()
    }

    /// Writes the file if it changed. A file that is never written says why each time a change
    /// is not saved.
    pub fn save_if_changed(&mut self) -> Result<(), String> {
        if !std::mem::take(&mut self.changed) {
            return Ok(());
        }
        if let Some(why) = &self.frozen {
            return Err(why.clone());
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let rest = toml::to_string(&self.table).map_err(|e| e.to_string())?;
        write_file(path, &format!("{HEADER}\nversion = {VERSION}\n{rest}"))
    }
}

/// A `session.state` value as the file keeps it: a number as the `f32` it is (0.3, not
/// 0.30000001192092896), unless it is whole; `None` for a null.
fn written(value: &Value) -> Option<toml::Value> {
    match value {
        Value::Null => None,
        Value::Number(n) if n.is_f64() => {
            let single = n.as_f64()? as f32;
            let shortest = single.to_string().parse::<f64>().ok()?;
            single.is_finite().then_some(toml::Value::Float(shortest))
        }
        _ => toml::Value::try_from(value).ok(),
    }
}

/// Two settings' values are the same: numbers as the `f32` the session holds.
pub fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x as f32).to_bits() == (y as f32).to_bits(),
        _ => a == b,
    }
}
