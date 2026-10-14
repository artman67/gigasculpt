//! Toggles show the session's state and brush sets follow the engine's brushes (decision
//! 0013): every lit button is lit exactly when `session.state` has the value it shows, after
//! every command a toggle runs; and a new preset with a family shows up on the shelf and the
//! wheel with no layout file touched; the brush section shows the active tool's settings.

use std::collections::BTreeSet;

use gs_engine::BrushInfo;
use serde_json::{Value, json};

use crate::rig::Ui;

/// A toggle on screen: where it is, what it runs, the state value it shows and whether it is
/// lit.
struct Toggle {
    at: String,
    step: Value,
    value: String,
    on: Value,
    lit: bool,
}

/// Every toggle `ui.describe` reports.
fn toggles(ui: &mut Ui) -> Vec<Toggle> {
    let d = ui.describe();
    let mut all = Vec::new();
    for panel in d["panels"].as_array().into_iter().flatten() {
        for w in panel["widgets"].as_array().into_iter().flatten() {
            let Some(value) = w["shows"]["value"].as_str() else {
                continue;
            };
            all.push(Toggle {
                at: format!("{} in {}", w["label"], panel["id"]),
                step: w["command"].clone(),
                value: value.to_string(),
                on: w["shows"]["on"].clone(),
                lit: w["selected"] == true,
            });
        }
    }
    all
}

/// `now` has the value `on`: numbers as numbers, so `on = 4` matches 4.0, and an object when
/// it holds `on`'s fields.
fn has(now: &Value, on: &Value) -> bool {
    if let (Value::Object(now), Value::Object(on)) = (now, on) {
        return on
            .iter()
            .all(|(k, v)| now.get(k).is_some_and(|x| has(x, v)));
    }
    let number = |v: &Value| v.as_f64().map(f64::to_bits);
    now == on || (number(now).is_some() && number(now) == number(on))
}

/// Checks that every toggle on screen is lit exactly when `session.state` has its value, and
/// returns them.
fn agree(ui: &mut Ui, after: &str) -> Vec<Toggle> {
    let all = toggles(ui);
    let state = ui.state();
    for t in &all {
        let now = &state[&t.value];
        let has = has(now, &t.on);
        assert_eq!(
            t.lit, has,
            "after {after}: {} shows {} = {}, and session.state has {now}",
            t.at, t.value, t.on
        );
    }
    all
}

#[test]
fn every_toggle_is_lit_exactly_when_the_state_has_its_value() {
    let mut ui = Ui::with_looks();
    // Tall enough to show the scene tray's sections that hold toggles, the others shut: what
    // scrolls out of sight is not listed.
    ui.host.lock().resize([1800, 1600], 1.0);
    ui.open_only(&["detail"]);
    let mut ran = BTreeSet::new();
    let mut values = BTreeSet::new();
    let right = json!({ "hand": "right" });
    // A part to look at: in an empty scene the view keeps no camera of its own, so the
    // projection would go back to perspective at the next call.
    ui.ok("primitive.sphere", json!({}));
    // Lit, so the Look options show the environments and the lights, which show only in the
    // modes they light (decision 0060 as amended for R1).
    ui.ok("view.display", json!({ "shading": "lit" }));
    ui.settle();
    ui.open_look();
    // The settings window shows one page at a time (decision 0052): each page in turn.
    for page in [
        "Pen",
        "Interface",
        "Navigation",
        "Cursor",
        "Files",
        "Geometry",
    ] {
        ui.ok("ui.settings", json!({ "open": true, "page": page }));
        ui.ok("ui.wheel", json!({ "open": true }));
        // Header > Window holds the hand and the panels' switches, and the Look panel (open
        // since before the first page) the view's. One opens at a time, so the Window menu
        // from the second page on; once All panels has hidden the header, it opens nothing.
        if page != "Pen" {
            ui.call("ui.menu", json!({ "open": "window" }));
        }
        let shown = agree(&mut ui, page);
        ui.ok("ui.wheel", json!({ "open": false }));
        let mut before = shown
            .iter()
            .map(|t| (t.at.clone(), t.lit))
            .collect::<Vec<_>>();
        for t in shown {
            values.insert(t.value.clone());
            // Each toggle's command once, as a click runs it; then every toggle agrees.
            if !ran.insert((t.value.clone(), t.on.to_string())) {
                continue;
            }
            let command = t.step["command"].as_str().unwrap_or_default();
            ui.ok(command, t.step["args"].clone());
            let was = before.iter().find(|b| b.0 == t.at).is_some_and(|b| b.1);
            let after = agree(&mut ui, &format!("{} ({})", t.step, t.at));
            // Its own command lights it, or switches it off if it was on (Perspective starts lit);
            // one that asks for a value first (Detail > Units) lights once the palette's form runs.
            let own = after.iter().find(|a| a.value == t.value && a.on == t.on);
            let asks = command == "ui.palette";
            assert!(
                asks || own.is_none_or(|o| o.lit || was),
                "{} changes or lights after its own command",
                t.at
            );
            before = after.iter().map(|a| (a.at.clone(), a.lit)).collect();
            ui.ok("ui.layout", right.clone());
        }
    }
    for value in [
        "brush",
        "symmetry.x",
        "look",
        "view.shading",
        "view.projection",
        "view.locked",
        "view.floor",
        "view.mask",
        "view.active_outline",
        "view.face_groups",
        "layout.hand",
        "session.paused",
        "view.transform",
        "view.environment",
        "light.1",
        "navigation.barrel_click",
        "navigation.classic",
        "layout.scale",
        "live_detail.mode",
    ] {
        assert!(
            values.contains(value),
            "some toggle shows {value}: {values:?}"
        );
    }
}

#[test]
fn a_new_preset_with_a_family_shows_on_the_shelf_and_the_wheel_without_touching_layout_files() {
    let text = "name = \"Select Wand\"\nid = \"brush.select-wand\"\nfamily = \"Mask\"\n\
                strength = 1.0\n[kernel]\ntype = \"attribute\"\ncolumn = \"mask\"\nop = \"set\"\n\
                value = 1.0\ngain = 8.0\n";
    let preset = gs_core::BrushPreset::from_toml(text).unwrap_or_else(|e| panic!("{e}"));
    let mut ui = Ui::new();
    // The engine lists brushes by family, then by name: Select Wand comes last.
    let info = BrushInfo::from(&preset);
    ui.host.lock().shell.directions.brushes.push(info);
    let picks = |w: &Value| w["command"]["args"]["brush"] == "Select Wand";
    ui.settle();
    ui.ok("ui.wheel", json!({ "open": true }));
    assert!(ui.widgets("wheel").iter().any(picks), "the wheel rings it");
    ui.ok("ui.wheel", json!({ "open": false }));
    // In its family, opened from the shelf, after the family's other brushes.
    ui.open_family("Mask");
    assert_eq!(
        shelves_with(&mut ui, picks),
        ["family"],
        "the shelf's Mask family shows it"
    );
    let labels = ui.labels("family");
    let at = |l: &str| labels.iter().position(|x| x == l);
    assert!(
        at("Mask Pen") < at("Select Rect") && at("Select Rect") < at("Select Wand"),
        "{labels:?}"
    );
    let rows = ui.ok("ui.palette", json!({ "query": "select wand" }))["rows"].clone();
    let row = (rows.as_array().into_iter().flatten()).find(|r| r["label"] == "Select Wand");
    let places = row.map(|r| r["places"].clone()).unwrap_or_default();
    assert!(
        places.to_string().contains("Brush shelf > Mask"),
        "the palette finds it on the shelf: {places}"
    );
}

/// The panels (not the wheel or the palette) with a widget `picks` matches.
fn shelves_with(ui: &mut Ui, picks: impl Fn(&Value) -> bool) -> Vec<String> {
    let d = ui.describe();
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let has = |p: &Value| {
        p["widgets"]
            .as_array()
            .is_some_and(|w| w.iter().any(&picks))
    };
    (panels.iter())
        .filter(|p| p["id"] != "wheel" && p["id"] != "palette" && has(p))
        .filter_map(|p| p["id"].as_str().map(String::from))
        .collect()
}

/// Brush tray > Brush shows the active tool's settings as the layout file's tools name them
/// (decision 0025, section 3, amended 2026-10-11): the brush's sliders, or while the Gizmo tool
/// is on the brush's Size titled Reach and its settings, then Pose's Lock and Sticky for the
/// gizmo shown; its size on screen is in Settings > Cursor (decision 0060, section 6).
#[test]
fn the_brush_section_shows_the_active_tools_settings() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    ui.settle();
    let shows = |ui: &mut Ui, label: &str| ui.labels("brush").iter().any(|l| l == label);
    // Softness, once Falloff here, is Tip's (decision 0060, section 6).
    for label in ["Size", "Strength", "Pressure…"] {
        assert!(shows(&mut ui, label), "the brush shows {label}");
    }
    assert!(!shows(&mut ui, "Reach"), "no Reach without the gizmo");
    ui.ok("ui.gizmo", json!({ "tool": true }));
    ui.settle();
    for label in ["Gizmo softness", "15°", "Reach", "Mask", "Lock", "Sticky"] {
        assert!(shows(&mut ui, label), "the gizmo tool shows {label}");
    }
    assert!(!shows(&mut ui, "Gizmo size"), "its size is a setting");
    assert!(!shows(&mut ui, "Strength"), "a gizmo drag uses no Strength");
}
