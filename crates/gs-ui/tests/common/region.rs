//! Where a part of the window is, as `ui.describe` reports it, so a test presses what it means
//! to press and keeps working when the layout moves things.

use serde_json::Value;

/// The rectangle of `name` in `ui.describe`'s answer `d`, in pixels: min x, min y, max x,
/// max y. `name` is a panel's id (`viewport` for the 3D view, `header`, `tray`, `status`), a
/// section's id (`masking`, the header's `look`), or a widget's label; `panel > name` looks only
/// in that panel. Panics, naming what it looked for, if nothing shows by that name.
pub fn region(d: &Value, name: &str) -> [f64; 4] {
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let (within, name) = match name.split_once(" > ") {
        Some((panel, rest)) => (Some(panel), rest),
        None => (None, name),
    };
    let panels: Vec<&Value> = (panels.iter())
        .filter(|p| within.is_none_or(|id| p["id"] == id))
        .collect();
    let list = |p: &Value, key: &str| p[key].as_array().cloned().unwrap_or_default();
    let panel = (within.is_none())
        .then(|| panels.iter().find(|p| p["id"] == name))
        .flatten()
        .map(|p| p["rect"].clone());
    // The first of the panels' `key` (sections or widgets) whose `field` is `name`.
    let named = |key: &str, field: &str| {
        (panels.iter().flat_map(|p| list(p, key)))
            .find(|x| x[field] == name)
            .map(|x| x["rect"].clone())
    };
    let rect = (panel.or_else(|| named("sections", "id"))).or_else(|| named("widgets", "label"));
    let ids: Vec<&Value> = panels.iter().map(|p| &p["id"]).collect();
    let rect = rect.unwrap_or_else(|| panic!("nothing called {name} shows in {ids:?}"));
    std::array::from_fn(|i| rect[i].as_f64().unwrap_or_default())
}

/// A rectangle's middle.
pub fn middle(r: [f64; 4]) -> [f64; 2] {
    [f64::midpoint(r[0], r[2]), f64::midpoint(r[1], r[3])]
}
