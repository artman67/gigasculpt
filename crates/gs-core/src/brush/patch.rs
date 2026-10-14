//! Patches: the settings changed for one brush, in the preset's own names, merged over its
//! preset. Resolving a brush is "preset, then patch", here and nowhere else (decision 0018).

use super::{BrushPreset, PresetError, StrokeSpec, carried, load};

/// Settings changed for one brush: a table in the preset's own names (`strength`, `modifier`,
/// `tip.focal`, `tip.alpha.image`, ...).
pub type Patch = toml::Table;

/// What makes a brush itself, which no patch may change.
const FIXED: [&str; 4] = ["name", "id", "family", "claims"];

/// `patch` merged into `base`: tables merge key by key, every other value (numbers, curves,
/// lists) replaces the one in `base`.
pub fn merge(base: &mut toml::Table, patch: &toml::Table) {
    for (key, value) in patch {
        match (base.get_mut(key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(p)) => merge(b, p),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

fn refused(message: String) -> PresetError {
    PresetError { message }
}

impl BrushPreset {
    /// The preset file `text` with `patch` merged in, read and checked like a file, so a bad
    /// value in the patch is refused with its key named. A patch may not change the brush's
    /// `name`, `id`, `family`, `claims` or `kernel.type`, nor the file's `version`; one naming
    /// another `stroke.class` keeps of the preset's stroke only what that class reads. The merged
    /// table is read as it stands, never written out and parsed again: a value of the wrong type
    /// or shape is refused at its key in the settings (`strength: invalid type: ...`), not at a
    /// line of a text no one wrote (review 0004, m2).
    pub fn resolve(text: &str, patch: &Patch) -> Result<BrushPreset, PresetError> {
        if let Some(key) = FIXED.iter().find(|k| patch.contains_key(**k)) {
            let why = format!("settings cannot change {key}: it is what makes the brush itself");
            return Err(refused(why));
        }
        if patch.get("kernel").and_then(|k| k.get("type")).is_some() {
            let why = "settings cannot change kernel.type: it is what makes the brush itself";
            return Err(refused(why.into()));
        }
        if patch.contains_key("version") {
            let why = "settings cannot change version: it is the file's format, not a setting";
            return Err(refused(why.into()));
        }
        if patch.is_empty() {
            return BrushPreset::from_toml(text);
        }
        let mut table: toml::Table = toml::from_str(text)
            .map_err(|e| refused(format!("brush preset does not parse: {e}")))?;
        restroke(&mut table, patch)?;
        merge(&mut table, patch);
        let read = serde_path_to_error::deserialize(toml::Value::Table(table));
        load::check(read.map_err(|e| shape(&e, patch))?)
    }
}

/// The preset `table`'s stroke made ready for a patch that names another stroke class: it keeps
/// only the settings that class reads ([`carried`]), and the patch's own go on top (decision
/// 0051, section 6). A gesture is a shape, a brush of its own, so no stroke changes to or from
/// one.
fn restroke(table: &mut toml::Table, patch: &Patch) -> Result<(), PresetError> {
    let named = patch.get("stroke").and_then(|s| s.get("class"));
    let Some(class) = named.and_then(toml::Value::as_str) else {
        return Ok(());
    };
    let stroke = match table.get("stroke") {
        Some(toml::Value::Table(t)) => t.clone(),
        _ => toml::Table::new(),
    };
    let was = stroke.get("class").and_then(toml::Value::as_str);
    let was = was.unwrap_or(StrokeSpec::default().class());
    if was == class {
        return Ok(());
    }
    if [was, class].contains(&"gesture") {
        let why = "stroke.class: a gesture is a shape, a brush of its own (Select Rect, Clip \
                   Curve), so no stroke changes to or from one";
        return Err(refused(why.into()));
    }
    table.insert("stroke".into(), toml::Value::Table(carried(&stroke, class)));
    Ok(())
}

/// A patch whose merge does not read as a preset, refused at the key the reader stopped on.
/// The shipped preset reads, so the patch is what is wrong there. A list in a patch replaces
/// the preset's whole list, so one written as "the preset's item, changed" lacks fields: the
/// refusal says so for the lists the patch holds where the reader stopped.
fn shape(e: &serde_path_to_error::Error<toml::de::Error>, patch: &Patch) -> PresetError {
    let at = e.path().to_string();
    let at = if at == "." {
        "settings".to_string()
    } else {
        at
    };
    let keys = |p: &str| -> Vec<String> {
        (p.split('.'))
            .map(|k| k.split('[').next().unwrap_or(k).to_string())
            .collect()
    };
    let stop = keys(&at);
    let mut found = Vec::new();
    lists(patch, "", &mut found);
    let near =
        (found.into_iter()).find(|l| keys(l).starts_with(&stop) || stop.starts_with(&keys(l)));
    let note = near.map_or(String::new(), |list| {
        format!("; settings.{list} replaces the preset's whole list, so write each item in full")
    });
    refused(format!("{at}: {}{note}", e.inner().message()))
}

/// The dotted keys of the lists of tables in `t`, under `at`.
fn lists(t: &toml::Table, at: &str, out: &mut Vec<String>) {
    for (k, v) in t {
        let key = if at.is_empty() {
            k.clone()
        } else {
            format!("{at}.{k}")
        };
        match v {
            toml::Value::Array(a) if a.iter().any(toml::Value::is_table) => out.push(key),
            toml::Value::Table(inner) => lists(inner, &key, out),
            _ => {}
        }
    }
}
