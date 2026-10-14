//! Saving a preset into its file's text (decision 0018, "Saving a brush"): only the settings
//! that differ are edited, in place, so comments, key order and layout survive and a preset
//! that equals its file saves that file byte for byte.

use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use super::{BrushPreset, Patch, PresetError};

/// What differs between two presets, setting by setting, in the preset's own names, as their
/// canonical forms say: a list is one setting, as in a patch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    /// Each changed or added setting, a table the first preset lacks as a whole.
    pub set: Patch,
    /// Each setting the second preset no longer has, by its dotted name (`tip.alpha`).
    pub removed: Vec<String>,
}

impl Changes {
    /// Whether the two presets mean the same.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.removed.is_empty()
    }

    /// Leaves out each changed setting the changes still mean the same without, as `same`
    /// says: a value a table gains that its default already means. A table added stays added.
    pub fn trim(&mut self, same: impl Fn(&Changes) -> bool) {
        let mut paths = Vec::new();
        leaves(&self.set, &[], &mut paths);
        for path in paths {
            let mut without = self.clone();
            take(&mut without.set, &path);
            if same(&without) {
                *self = without;
            }
        }
    }
}

impl BrushPreset {
    /// The settings that take this preset to `to`.
    pub fn changes(&self, to: &BrushPreset) -> Changes {
        let mut out = Changes::default();
        diff(&canonical(self), &canonical(to), &[], &mut out);
        out
    }

    /// The preset file `text` edited so it reads as this preset: each setting that differs is
    /// replaced where it stands (keeping its comment), added to its table, or removed, and
    /// every other byte is kept; a table the file lacks gets only the settings its defaults do
    /// not already mean. A preset that equals its file gives the file unchanged. The result is
    /// read back, and refused unless it means exactly this preset.
    ///
    /// `named` lists dotted settings the result must write even where the file's default means
    /// the same: naming a setting can matter beside its value (a size curve named keeps the
    /// session's global one off, decision 0036).
    pub fn write_into(&self, text: &str, named: &[&str]) -> Result<String, PresetError> {
        let mut changes = BrushPreset::from_toml(text)?.changes(self);
        let reads_as = |out: Result<String, PresetError>| {
            out.and_then(|out| BrushPreset::from_toml(&out))
                .is_ok_and(|p| p == *self)
        };
        changes.trim(|c| reads_as(edited(text, c)));
        let file: toml::Table = toml::from_str(text).unwrap_or_default();
        let mine = canonical(self);
        for name in named {
            let path: Vec<&str> = name.split('.').collect();
            if let (None, Some(v)) = (at(&file, &path), at(&mine, &path)) {
                put(&mut changes.set, &path, v.clone());
            }
        }
        if changes.is_empty() {
            return Ok(text.to_string());
        }
        let out = edited(text, &changes)?;
        match BrushPreset::from_toml(&out) {
            Ok(p) if p == *self => Ok(out),
            Ok(_) => Err(refused(format!(
                "{}: the edited file does not read as the brush",
                self.name
            ))),
            Err(e) => Err(refused(format!("{}: the edited file {e}", self.name))),
        }
    }
}

fn refused(message: String) -> PresetError {
    PresetError { message }
}

/// The preset file `text` with `changes` made where it writes them.
fn edited(text: &str, changes: &Changes) -> Result<String, PresetError> {
    let mut doc: DocumentMut =
        (text.parse()).map_err(|e| refused(format!("brush preset does not parse: {e}")))?;
    set(doc.as_table_mut(), &changes.set, false);
    for name in &changes.removed {
        remove(doc.as_table_mut(), &name.split('.').collect::<Vec<_>>());
    }
    Ok(doc.to_string())
}

/// The value at the dotted `path` in `t`, if `t` has it.
fn at<'a>(t: &'a toml::Table, path: &[&str]) -> Option<&'a toml::Value> {
    let (last, tables) = path.split_last()?;
    let mut t = t;
    for key in tables {
        t = t.get(*key)?.as_table()?;
    }
    t.get(*last)
}

/// `changes` merged into the file's table `t` as a patch merges (tables key by key, any other
/// value replaced whole); `inline` when `t` is written inside braces.
fn set(t: &mut dyn TableLike, changes: &toml::Table, inline: bool) {
    for (key, v) in changes {
        // Tables inside braces or written as dotted keys take their new tables the same way.
        let braces = match t.get(key) {
            Some(Item::Table(file)) => Some(inline || file.is_dotted()),
            Some(Item::Value(Value::InlineTable(_))) => Some(true),
            _ => None,
        };
        if let (Some(braces), toml::Value::Table(sub)) = (braces, v)
            && let Some(inner) = t.get_mut(key).and_then(Item::as_table_like_mut)
        {
            set(inner, sub, braces);
            continue;
        }
        if let Some(Item::Value(old)) = t.get_mut(key) {
            let decor = old.decor().clone();
            *old = value(v);
            *old.decor_mut() = decor;
            continue;
        }
        let item = match v {
            toml::Value::Table(sub) if !inline => Item::Table(table(sub)),
            _ => Item::Value(value(v)),
        };
        t.insert(key, item);
    }
}

/// A new table, written as a `[header]` section unless it only holds tables.
fn table(t: &toml::Table) -> Table {
    let mut out = Table::new();
    set(&mut out, t, false);
    out.set_implicit(!t.is_empty() && t.values().all(toml::Value::is_table));
    out
}

/// A value as the shipped presets write it: lists and tables inside brackets and braces.
fn value(v: &toml::Value) -> Value {
    let mut out = match v {
        toml::Value::String(s) => Value::from(s.as_str()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => Value::from(*f),
        toml::Value::Boolean(b) => Value::from(*b),
        toml::Value::Datetime(d) => Value::from(*d),
        toml::Value::Array(a) => Value::Array(a.iter().map(value).collect()),
        toml::Value::Table(t) => {
            Value::InlineTable(t.iter().map(|(k, v)| (k.as_str(), value(v))).collect())
        }
    };
    match &mut out {
        Value::Array(a) => a.fmt(),
        Value::InlineTable(t) => t.fmt(),
        _ => {}
    }
    out
}

/// The setting at `path` taken out of the file's table `t`, where the file writes it.
fn remove(t: &mut dyn TableLike, path: &[&str]) {
    match path {
        [key] => {
            t.remove(key);
        }
        [key, rest @ ..] => {
            if let Some(inner) = t.get_mut(key).and_then(Item::as_table_like_mut) {
                remove(inner, rest);
            }
        }
        [] => {}
    }
}

/// The canonical form as a table: every default filled in, floats as their shortest decimals.
fn canonical(p: &BrushPreset) -> toml::Table {
    toml::from_str(&p.canonical()).unwrap_or_default()
}

/// The settings that take `from` to `to`, the tables at `at`, added to `out`.
fn diff<'a>(from: &'a toml::Table, to: &'a toml::Table, at: &[&'a str], out: &mut Changes) {
    for (k, v) in to {
        let path = [at, &[k.as_str()]].concat();
        match (from.get(k), v) {
            (Some(toml::Value::Table(a)), toml::Value::Table(b)) => diff(a, b, &path, out),
            (Some(a), b) if a == b => {}
            _ => put(&mut out.set, &path, v.clone()),
        }
    }
    for k in from.keys().filter(|k| !to.contains_key(*k)) {
        out.removed.push([at, &[k.as_str()]].concat().join("."));
    }
}

/// The path of each value in `t` that is not a table, under `at`: each setting, a list being
/// one.
fn leaves(t: &toml::Table, at: &[String], out: &mut Vec<Vec<String>>) {
    for (k, v) in t {
        let path = [at, std::slice::from_ref(k)].concat();
        match v {
            toml::Value::Table(sub) => leaves(sub, &path, out),
            _ => out.push(path),
        }
    }
}

/// `t` without the value at `path`; the tables on the way stay.
fn take(t: &mut toml::Table, path: &[String]) {
    match path {
        [key] => {
            t.remove(key);
        }
        [key, rest @ ..] => {
            if let Some(toml::Value::Table(inner)) = t.get_mut(key) {
                take(inner, rest);
            }
        }
        [] => {}
    }
}

/// `value` at the dotted `path` in `t`, making the tables on the way.
fn put(t: &mut toml::Table, path: &[&str], value: toml::Value) {
    match path {
        [] => {}
        [key] => {
            t.insert((*key).to_string(), value);
        }
        [key, rest @ ..] => {
            let inner = t
                .entry((*key).to_string())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if let toml::Value::Table(inner) = inner {
                put(inner, rest, value);
            }
        }
    }
}
