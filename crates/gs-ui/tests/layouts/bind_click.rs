//! Binding a key from a control (catalog `interface.gesture-map-widgets`, decision 0025
//! section 5 as amended 2026-10-12): Ctrl+Alt + click on a button aims at it without running
//! it and the status line asks for the key; the next key is bound to the button's command, a
//! taken one is asked about naming its holder and moves once `replace` is confirmed, Escape binds
//! nothing, a switch's key switches it, and a wheel notch with a modifier held binds as a key.

use gs_platform::{Input, Modifiers};
use serde_json::json;

use crate::keymap::runs;
use crate::region::middle;
use crate::rig::{Ui, held};

/// Ctrl and Alt held.
fn both() -> Modifiers {
    Modifiers {
        ctrl: true,
        alt: true,
        ..Modifiers::default()
    }
}

/// A Ctrl+Alt press on the scene tray's control `label`, then Ctrl and Alt let go.
fn aim(ui: &mut Ui, label: &str) {
    aim_at(ui, &format!("tray > {label}"));
}

/// A Ctrl+Alt press on the widget `name`, then Ctrl and Alt let go.
fn aim_at(ui: &mut Ui, name: &str) {
    let [x, y] = middle(ui.rect(name));
    ui.pen(x, y);
    ui.input(&[], both());
    ui.tap_at(x, y);
    ui.input(&[], Modifiers::default());
}

/// The status line's words, joined.
fn status(ui: &mut Ui) -> String {
    ui.labels("status").join(" | ")
}

/// A window with a sphere and the scene tray showing Detail, where Subdivide is.
fn detail() -> Ui {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    ui.open_only(&["detail"]);
    ui
}

/// Catalog `interface.gesture-map-widgets`: "Ctrl+Alt + click on a button and then K binds K
/// to that button's command".
#[test]
fn ctrl_alt_click_on_a_button_then_k_binds_k_to_its_command() {
    let mut ui = detail();
    assert_eq!(runs(&ui, "K"), None, "K starts free");
    aim(&mut ui, "Subdivide");
    assert!(
        ui.logged("level.subdivide").is_empty(),
        "the aiming press runs nothing"
    );
    let asks = "Press a key for Subdivide (Escape cancels)";
    assert!(status(&mut ui).contains(asks), "{}", status(&mut ui));
    ui.tap("K", Modifiers::default());
    assert_eq!(
        runs(&ui, "K").as_deref(),
        Some("level.subdivide"),
        "K is bound"
    );
    assert!(!status(&mut ui).contains(asks), "the prompt goes");
    assert!(
        ui.logged("level.subdivide").is_empty(),
        "binding K ran nothing"
    );
    ui.tap("K", Modifiers::default());
    assert_eq!(ui.logged("level.subdivide").len(), 1, "K runs Subdivide");
    // A plain tap on the button runs it, as always.
    ui.press("tray > Subdivide");
    assert_eq!(ui.logged("level.subdivide").len(), 2);
}

/// A taken key: the palette asks to replace it, naming the holder in the question's line;
/// confirmed, the key moves. Escape binds nothing.
#[test]
fn a_taken_key_names_its_holder_and_escape_binds_nothing() {
    let mut ui = detail();
    aim(&mut ui, "Subdivide");
    ui.tap("I", held("Ctrl"));
    let named = "Ctrl+I runs Invert mask in the window";
    let asked = ui.labels("palette");
    assert!(asked.iter().any(|l| l.contains(named)), "{asked:?}");
    assert_eq!(runs(&ui, "Ctrl+I").as_deref(), Some("mask.invert"));
    let form = ui.host.lock().shell.palette.form.as_ref().map(|f| {
        let fields: Vec<String> = f.fields.iter().map(|(n, _)| n.clone()).collect();
        (f.step.command.clone(), fields)
    });
    assert_eq!(form, Some(("ui.keymap".into(), vec!["replace".into()])));
    ui.press("palette > Replace");
    assert_eq!(
        runs(&ui, "Ctrl+I").as_deref(),
        Some("level.subdivide"),
        "the key moved"
    );

    aim(&mut ui, "Subdivide");
    ui.tap("Escape", Modifiers::default());
    assert!(!status(&mut ui).contains("Press a key"), "Escape cancels");
    ui.tap("J", Modifiers::default());
    assert_eq!(runs(&ui, "J"), None, "nothing was bound");
}

/// A switch's key switches it, as its control does; a wheel notch with Alt held binds that
/// notch, and turning it runs the command without moving the camera.
#[test]
fn a_switch_and_a_wheel_notch_bind_as_a_button_and_a_key_do() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let one_open = |ui: &mut Ui| ui.state()["layout.one_open"].clone();
    assert_eq!(one_open(&mut ui), json!(true));
    crate::header::open(&mut ui, "Window");
    aim_at(&mut ui, "menu > One section open");
    assert_eq!(one_open(&mut ui), json!(true), "aiming switched nothing");
    ui.tap("J", Modifiers::default());
    ui.tap("J", Modifiers::default());
    assert_eq!(one_open(&mut ui), json!(false), "J switches it off");
    ui.tap("J", Modifiers::default());
    assert_eq!(one_open(&mut ui), json!(true), "and on again");
    // A notch bound to it switches it too.
    crate::header::open(&mut ui, "Window");
    aim_at(&mut ui, "menu > One section open");
    let down = [Input::Wheel { dx: 0.0, dy: -1.0 }];
    ui.input(&down, held("Alt"));
    ui.input(&down, held("Alt"));
    assert_eq!(one_open(&mut ui), json!(false), "Alt+WheelDown switches it");

    // Shut the menu, or the click that aims at Subdivide only shuts it.
    ui.ok("ui.menu", json!({ "open": false }));
    ui.open_only(&["detail"]);
    aim(&mut ui, "Subdivide");
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let camera = |ui: &mut Ui| ui.ok("view.get", json!({ "view": "main" }))["camera"].to_string();
    let seen = camera(&mut ui);
    ui.input(&[Input::Wheel { dx: 0.0, dy: 1.0 }], held("Alt"));
    assert_eq!(runs(&ui, "Alt+WheelUp").as_deref(), Some("level.subdivide"));
    assert!(
        ui.logged("level.subdivide").is_empty(),
        "binding ran nothing"
    );
    ui.input(&[Input::Wheel { dx: 0.0, dy: 1.0 }], held("Alt"));
    assert_eq!(ui.logged("level.subdivide").len(), 1, "the notch runs it");
    assert_eq!(camera(&mut ui), seen, "the camera stays");
}

/// The keys sheet says how, where a person looks for keys (UI-keys, 2026-10-14).
#[test]
fn the_keys_sheet_says_ctrl_alt_click_binds_a_key() {
    let mut ui = Ui::new();
    ui.ok("ui.settings", json!({ "open": true, "page": "Keys" }));
    ui.settle();
    let line = "Or hold Ctrl+Alt and click a button to give it a key.";
    let labels = ui.labels("settings");
    assert!(labels.iter().any(|l| l == line), "{labels:?}");
}
