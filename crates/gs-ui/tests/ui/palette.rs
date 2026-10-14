//! The interface driven headlessly: the palette key opens the palette through `ui.palette`,
//! typing narrows it, Enter or a click runs the command on the engine; the pause toggle and
//! the keymap reach the engine the same way.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use gs_protocol::{ErrorCode, Outcome};
use gs_ui::{Binding, Keymap};
use serde_json::json;

use crate::rig::{Rig, agent, harness, window};

#[test]
fn the_palette_key_opens_it_typing_finds_a_command_and_enter_runs_it() {
    let mut h = harness(Rig::new());
    h.key_press(Key::Space);
    h.run();
    assert!(h.state().host.lock().shell.palette.open, "Space opened it");
    h.event(Event::Text("sphere".into()));
    h.run();
    let first = {
        let studio = h.state().host.lock();
        let panels = studio.shell.panels();
        let rows = (panels.iter().filter(|p| p.id == "palette")).flat_map(|p| &p.widgets);
        let mut rows = rows.filter(|w| w.role == "list item");
        rows.next().map(|w| w.label.clone())
    };
    // The control a person presses leads, Header > Add's Sphere (review 0009, C12).
    assert_eq!(first.as_deref(), Some("Sphere"), "the best match");
    h.key_press(Key::Enter);
    h.run();
    assert!(
        h.state_mut().has_part("sphere"),
        "Enter ran primitive.sphere"
    );
    let studio = h.state().host.lock();
    assert!(!studio.shell.palette.open, "running a command closes it");
    let names: Vec<&str> = studio.shell.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["sphere"], "the parts panel heard");
}

#[test]
fn clicking_a_row_runs_its_command_and_escape_closes() {
    let mut h = harness(Rig::new());
    h.state_mut()
        .ok(&window(), "ui.palette", json!({ "query": "cube" }));
    h.run();
    h.get_by_label("primitive.cube").click();
    h.run();
    assert!(
        h.state_mut().has_part("cube"),
        "the click ran primitive.cube"
    );
    h.state_mut()
        .ok(&agent(), "ui.palette", json!({ "query": "zzqx" }));
    h.run();
    h.get_by_label("No command matches.");
    h.key_press(Key::Escape);
    h.run();
    assert!(
        !h.state().host.lock().shell.palette.open,
        "Escape closed it"
    );
}

#[test]
fn arrow_keys_pick_the_row_enter_runs() {
    let mut h = harness(Rig::new());
    let info = h
        .state_mut()
        .ok(&window(), "ui.palette", json!({ "query": "primitive" }));
    assert_eq!(
        info["matches"],
        json!(["primitive.cube", "primitive.sphere", "primitive.cylinder"]),
        "equal scores: by name"
    );
    h.run();
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowUp);
    h.key_press(Key::ArrowDown);
    h.run();
    h.key_press(Key::Enter);
    h.run();
    let rig = h.state_mut();
    assert!(
        rig.has_part("cylinder") && !rig.has_part("sphere") && !rig.has_part("cube"),
        "the last row ran"
    );
}

#[test]
fn the_pause_toggle_reaches_the_engine_and_shows_its_state() {
    let mut h = harness(Rig::new());
    h.get_by_label("Pause agents").click();
    h.run();
    let info = h.state_mut().ok(&agent(), "session.state", json!({}));
    assert_eq!(
        info["session.paused"],
        json!(true),
        "the engine paused agents"
    );
    let refused = h.state_mut().call(&agent(), "primitive.sphere", json!({}));
    let Outcome::Error(e) = refused.outcome else {
        panic!("an agent edit while paused must fail")
    };
    assert_eq!(e.code, ErrorCode::Paused);
    h.get_by_label("Agents paused").click();
    h.run();
    let info = h.state_mut().ok(&agent(), "session.state", json!({}));
    assert_eq!(info["session.paused"], json!(false), "and resumed them");
    h.get_by_label("Pause agents");
}

#[test]
fn bound_keys_run_their_commands_and_the_palette_shows_them() {
    let mut keymap = Keymap::default();
    keymap.bindings.push(Binding {
        key: "Ctrl+Shift+K".into(),
        command: "primitive.sphere".into(),
        args: toml::from_str("subdivisions = 2").unwrap_or_else(|e| panic!("{e}")),
    });
    let mut h = harness(Rig::with_keymap(keymap));
    let ctrl_shift = Modifiers {
        ctrl: true,
        shift: true,
        command: true,
        ..Modifiers::NONE
    };
    h.key_press_modifiers(ctrl_shift, Key::K);
    h.run();
    let stats = h
        .state_mut()
        .ok(&agent(), "mesh.stats", json!({ "part": "sphere" }));
    assert_eq!(
        stats["parts"][0]["faces"],
        json!(24),
        "the binding's arguments were used"
    );
    h.key_press_modifiers(
        Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        },
        Key::Z,
    );
    h.run();
    assert!(!h.state_mut().has_part("sphere"), "Ctrl+Z ran history.undo");
    h.state_mut()
        .ok(&window(), "ui.palette", json!({ "query": "history.undo" }));
    h.run();
    h.get_by_label("history.undo    [Ctrl+Z]");
    let row = widgets(h.state_mut(), "palette")
        .into_iter()
        .find(|w| w["label"] == "history.undo")
        .unwrap_or_default();
    assert_eq!(
        row["key"],
        json!("Ctrl+Z"),
        "describe reports the key: {row}"
    );
}

#[test]
fn agent_activity_shows_in_the_status_bar() {
    let mut h = harness(Rig::new());
    // Wide enough for the whole line: a narrower window leaves out its end, the session's name
    // and the revision among it (decision 0016, section 17, item 10).
    h.set_size(egui::vec2(1600.0, 700.0));
    h.run();
    h.get_by_label("Agent: no agent activity");
    h.get_by_label("Session main");
    h.state_mut().ok(&agent(), "primitive.cube", json!({}));
    h.run();
    h.get_by_label("Agent: claude#9 ran primitive.cube (done)");
    h.get_by_label("rev 1");
    let _ = h
        .state_mut()
        .call(&agent(), "part.get", json!({ "part": "nope" }));
    h.run();
    h.get_by_label("Agent: claude#9 ran part.get (refused)");
}

/// The widgets of panel `id` in the window's own description.
fn widgets(rig: &mut Rig, id: &str) -> Vec<serde_json::Value> {
    let d = rig.ok(&window(), "ui.describe", json!({}));
    let panel = d["panels"]
        .as_array()
        .and_then(|p| p.iter().find(|p| p["id"] == id).cloned())
        .unwrap_or_default();
    panel["widgets"].as_array().cloned().unwrap_or_default()
}

#[test]
fn reopening_keeps_the_highlighted_row_and_describe_shows_it() {
    let mut h = harness(Rig::new());
    h.state_mut()
        .ok(&window(), "ui.palette", json!({ "query": "primitive" }));
    h.run();
    h.key_press(Key::ArrowDown);
    h.run();
    h.state_mut().ok(&window(), "ui.palette", json!({}));
    h.run();
    assert_eq!(
        h.state().host.lock().shell.palette.selected,
        1,
        "still the second row"
    );
    let rows: Vec<(String, bool)> = widgets(h.state_mut(), "palette")
        .into_iter()
        .filter(|w| w["role"] == "list item")
        .map(|w| {
            (
                w["label"].as_str().unwrap_or_default().to_string(),
                w["selected"] == true,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("primitive.cube".to_string(), false),
            ("primitive.sphere".to_string(), true),
            ("primitive.cylinder".to_string(), false)
        ]
    );
}

#[test]
fn the_toggle_shows_its_state_and_has_no_key_unless_bound() {
    let mut rig = Rig::new();
    let toggle = |rig: &mut Rig| {
        widgets(rig, "status")
            .into_iter()
            .find(|w| w["role"] == "toggle")
            .unwrap_or_default()
    };
    let off = toggle(&mut rig);
    assert_eq!(
        (off["selected"].clone(), off["key"].clone()),
        (json!(false), json!(null))
    );
    rig.ok(&window(), "session.control", json!({ "paused": true }));
    let on = toggle(&mut rig);
    assert_eq!(on["selected"], json!(true), "on while agents are paused");
    assert_eq!(
        on["command"]["args"],
        json!({ "paused": false }),
        "and it would resume them"
    );
}

#[test]
fn keys_stay_with_the_open_palette_even_when_its_field_loses_focus() {
    let mut h = harness(Rig::new());
    h.state_mut().ok(&agent(), "primitive.cube", json!({}));
    h.state_mut()
        .ok(&window(), "ui.palette", json!({ "query": "zzqx" }));
    h.run();
    // A click on what the palette says, which runs nothing, away from its field: a click
    // outside the palette shuts it (review 0009, C9).
    let away = h.get_by_label("No command matches.").rect().center();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: away,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run();
    }
    assert!(
        !h.ctx.egui_wants_keyboard_input(),
        "the click took the focus away"
    );
    let ctrl = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    };
    h.key_press_modifiers(ctrl, Key::Z);
    h.run();
    assert!(
        h.state_mut().has_part("cube"),
        "Ctrl+Z did not undo behind the palette"
    );
}

#[test]
fn the_humans_own_calls_are_not_agent_activity() {
    let mut h = harness(Rig::new());
    h.state_mut().ok(&window(), "primitive.cube", json!({}));
    h.run();
    h.get_by_label("Agent: no agent activity");
    h.get_by_label("rev 1");
}
