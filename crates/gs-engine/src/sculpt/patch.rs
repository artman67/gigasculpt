//! Settings patches as calls carry them (decision 0018): JSON in the preset's own names, merged
//! into the session's patch for a brush, and resolved over the brush's preset by gs-core's
//! [`BrushPreset::resolve`], the one place that says what a brush's settings are.

use gs_core::{BrushPreset, PresetError};
use gs_protocol::{CommandError, Violation};
use serde_json::{Map, Value, json};

use super::presets::Shipped;
use crate::errors;

/// A refused setting at JSON pointer `path`: `E_ARGS_INVALID`, listed as a violation of that
/// argument like a schema's.
pub(crate) fn refused(path: &str, message: String) -> CommandError {
    CommandError {
        violations: vec![Violation {
            path: path.into(),
            message: message.clone(),
        }],
        ..errors::args_invalid(path, message)
    }
}

/// Settings in the preset's own names, as a call writes them, for example
/// `{ "strength": 0.6, "tip": { "focal": 40, "alpha": { "image": "square" } } }`.
pub type Settings = Map<String, Value>;

/// The shortcut arguments and the settings key each one stands for.
pub(crate) const SHORTCUTS: [(&str, &[&str]); 5] = [
    ("strength", &["strength"]),
    ("falloff", &["tip", "focal"]),
    ("modifier", &["modifier"]),
    ("alpha", &["tip", "alpha", "image"]),
    ("stroke", &["stroke", "class"]),
];

/// `incoming` merged into `stored`: objects merge key by key, `null` drops a key (so the
/// preset's own value applies again), everything else replaces. Objects left empty go.
pub(crate) fn merge(stored: &mut Settings, incoming: &Settings) {
    for (key, value) in incoming {
        match value {
            Value::Null => {
                stored.remove(key);
            }
            Value::Object(inner) => {
                let mut sub = match stored.remove(key) {
                    Some(Value::Object(o)) => o,
                    _ => Map::new(),
                };
                merge(&mut sub, inner);
                if !sub.is_empty() {
                    stored.insert(key.clone(), Value::Object(sub));
                }
            }
            _ => {
                stored.insert(key.clone(), value.clone());
            }
        }
    }
}

/// The value at `path` in `s`, if any.
pub(crate) fn get<'a>(s: &'a Settings, path: &[&str]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    rest.iter().try_fold(s.get(*first)?, |v, k| v.get(k))
}

/// `s` with `value` at `path`, making the objects on the way.
pub(crate) fn put(s: &mut Settings, path: &[&str], value: Value) {
    let Some((last, ways)) = path.split_last() else {
        return;
    };
    let mut at = s;
    for k in ways {
        let next = at
            .entry((*k).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !next.is_object() {
            *next = Value::Object(Map::new());
        }
        let Value::Object(o) = next else { return };
        at = o;
    }
    at.insert((*last).to_string(), value);
}

/// A number as a call would write it: the shortest decimal that reads back as `x`.
pub(crate) fn number(x: f32) -> Value {
    let shortest: f64 = x.to_string().parse().unwrap_or(f64::from(x));
    serde_json::Number::from_f64(shortest).map_or(Value::Null, Value::Number)
}

/// `settings` with the shortcut arguments that were given (`(name, value)`) written into their
/// keys. Giving a key both ways is refused, naming the shortcut.
pub(crate) fn with_shortcuts(
    settings: Option<&Settings>,
    given: &[(&str, Value)],
) -> Result<Settings, CommandError> {
    let mut out = settings.cloned().unwrap_or_default();
    for (name, value) in given {
        let Some((_, path)) = SHORTCUTS.iter().find(|s| s.0 == *name) else {
            continue;
        };
        if get(&out, path).is_some() {
            let key = path.join(".");
            let message = format!("give {name} or settings.{key}, not both");
            return Err(refused(&format!("/{name}"), message));
        }
        put(&mut out, path, value.clone());
    }
    Ok(out)
}

/// The JSON pointer of the argument a refused setting came from: its shortcut when the call
/// gave one for it, else its place in `settings`.
fn pointer(e: &PresetError, shortcuts: &[&str]) -> String {
    let key: String = (e.message.chars())
        .take_while(|c| c.is_ascii_lowercase() || *c == '_' || *c == '.')
        .collect();
    let path: Vec<&str> = key.split('.').collect();
    let shortcut = SHORTCUTS
        .iter()
        .find(|(name, p)| shortcuts.contains(name) && *p == path.as_slice());
    match shortcut {
        Some((name, _)) => format!("/{name}"),
        None if SETTINGS.contains(&path[0]) => format!("/settings/{}", path.join("/")),
        None => "/settings".into(),
    }
}

/// The preset's settings by their top-level names: what a `settings` argument may hold.
const SETTINGS: [&str; 13] = [
    "strength",
    "modifier",
    "mode",
    "kernel",
    "tip",
    "depth",
    "sampling",
    "stroke",
    "pressure",
    "post",
    "automask",
    "paint",
    "live_detail",
];

/// The pressure curves the session's global set may hold.
pub(crate) const CURVES: [&str; 5] = ["size", "strength", "spacing", "modifier", "imbed"];

/// An object with only `keys` at its top level (any value under each), or null.
fn keyed(keys: &[&str]) -> schemars::Schema {
    let props: Map<String, Value> = keys.iter().map(|k| ((*k).into(), json!({}))).collect();
    schemars::json_schema!({
        "type": ["object", "null"],
        "properties": props,
        "additionalProperties": false,
    })
}

/// The schema of a `settings` argument: the preset's settings by their top-level names; what
/// each holds is checked like a preset file.
pub(crate) fn settings_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    keyed(&SETTINGS)
}

/// The schema of `global_pressure`: curves by name.
pub(crate) fn curves_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    keyed(&CURVES)
}

/// `shipped` with `settings` merged in, checked like a preset file. A refused setting is
/// `E_ARGS_INVALID` at the argument it came from (`shortcuts` lists the shortcut arguments the
/// call gave).
pub(crate) fn resolve(
    shipped: &Shipped,
    settings: &Settings,
    shortcuts: &[&str],
) -> Result<BrushPreset, CommandError> {
    if settings.is_empty() {
        return Ok(shipped.preset.clone());
    }
    let Ok(toml::Value::Table(table)) = toml::Value::try_from(settings) else {
        let message = "settings must be a table of preset settings, without nulls";
        return Err(refused("/settings", message.into()));
    };
    BrushPreset::resolve(&shipped.text, &table).map_err(|e| {
        let message = format!("{}: {}", shipped.preset.name, e.message);
        refused(&pointer(&e, shortcuts), message)
    })
}
