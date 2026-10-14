//! Bind a key as one gesture (review 0009, C11; decision 0025, section 5, amended 2026-10-14):
//! Settings > Keys' Bind a key… lists what a key can run, and once one is picked says "Press
//! the keys for" it, takes the next chord, shows it, and binds it on Bind; a taken chord asks
//! Replace, as every taken key does. Typing the key's name stays `ui.keymap`'s, for agents.

use gs_platform::{Input, Modifiers};
use serde_json::json;

use crate::keymap::runs;
use crate::rig::Ui;

/// Ctrl and Shift held.
fn ctrl_shift() -> Modifiers {
    Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::default()
    }
}

/// Settings > Keys > Bind a key…, then Quick save picked from the palette's list by typing its
/// name: the panel asks for the keys.
fn bind_quick_save(ui: &mut Ui) {
    ui.ok("ui.settings", json!({ "open": true, "page": "Keys" }));
    ui.settle();
    ui.wait(0.1);
    ui.press("settings > Bind a key…");
    let asked = ui.labels("palette");
    assert!(
        asked.iter().any(|l| l == "Pick what the key runs"),
        "{asked:?}"
    );
    ui.input(&[], Modifiers::default());
    ui.input(&[Input::Text("quick save".into())], Modifiers::default());
    ui.press("palette > Quick save");
    let asked = ui.labels("palette");
    assert!(
        asked.iter().any(|l| l == "Press the keys for Quick save"),
        "{asked:?}"
    );
}

/// Review 0009, C11: Bind a key… takes the chord pressed, shows it, and Bind binds it; the
/// chord runs nothing while it is taken.
#[test]
fn bind_a_key_by_pressing_a_chord() {
    let mut ui = Ui::new();
    bind_quick_save(&mut ui);
    ui.tap("K", ctrl_shift());
    let asked = ui.labels("palette");
    assert!(asked.iter().any(|l| l == "Ctrl+Shift+K"), "{asked:?}");
    assert_eq!(runs(&ui, "Ctrl+Shift+K"), None, "not before Bind");
    ui.press("palette > Bind");
    assert_eq!(
        runs(&ui, "Ctrl+Shift+K").as_deref(),
        Some("file.quick_save")
    );
    assert!(ui.labels("palette").is_empty(), "the panel shuts");
}

/// A chord goes to the one thing in front (decision 0025, section 5, amended 2026-10-14,
/// UI-last): pressed with the recovery question over the panel it binds nothing, and once the
/// dialog is put away the panel takes the next.
#[test]
fn a_chord_with_the_recovery_question_in_front_binds_nothing() {
    let dir = crate::rig::folder("bind-behind-recovery");
    crate::recovery::crash(&dir);
    let mut ui = crate::recovery::launched(&dir);
    bind_quick_save(&mut ui);
    ui.press("recovery > Discard");
    let asked = crate::recovery::buttons(&mut ui);
    assert_eq!(asked, ["Delete", "Cancel"], "the question is asked, on top");
    ui.tap("K", ctrl_shift());
    let shown = ui.labels("palette");
    assert!(shown.iter().any(|l| l == "Waiting for a key…"), "{shown:?}");
    ui.press("recovery > Cancel");
    ui.press("recovery > Later");
    ui.tap("K", ctrl_shift());
    let shown = ui.labels("palette");
    assert!(shown.iter().any(|l| l == "Ctrl+Shift+K"), "{shown:?}");
}

/// Review 0009, C11: a taken chord asks Replace, naming what holds it by title; Replace moves
/// the key.
#[test]
fn a_taken_chord_asks_replace() {
    let mut ui = Ui::new();
    bind_quick_save(&mut ui);
    ui.tap(
        "D",
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
    );
    ui.press("palette > Bind");
    let asked = ui.labels("palette");
    let named = "Ctrl+D runs Subdivide in the window";
    assert!(asked.iter().any(|l| l.contains(named)), "{asked:?}");
    assert_eq!(runs(&ui, "Ctrl+D").as_deref(), Some("level.subdivide"));
    ui.press("palette > Replace");
    assert_eq!(runs(&ui, "Ctrl+D").as_deref(), Some("file.quick_save"));
}
