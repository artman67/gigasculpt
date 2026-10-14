//! Quitting and deleting from the keyboard (review 0009, R10 and R12): File > Quit and Ctrl+Q
//! close the window as its close button does, asking first while there are unsaved changes;
//! Delete deletes the active part as Part > Delete > This part does, asking nothing, and so does
//! Backspace, a Mac laptop's delete key, except in a text field, which keeps it. X still turns X
//! symmetry on and off (tests/layouts/symmetry.rs).

use gs_platform::Modifiers;
use serde_json::json;

use crate::paint::type_hex;
use crate::rig::{Ui, held};
use crate::scene::{parts, press, question};

/// Whether the window closes now.
fn closes(ui: &Ui) -> bool {
    ui.host.lock().shell.closes()
}

/// Review 0009, R10: with unsaved changes, Ctrl+Q asks Save, Don't save or Cancel, as closing
/// the window does, and Don't save quits; File > Quit asks the same, and with nothing to lose
/// it quits at once.
#[test]
fn ctrl_q_and_file_quit_ask_first_with_unsaved_changes() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.tap("Q", held("Ctrl"));
    assert_eq!(ui.labels("palette"), question("closing", "this scene"));
    assert!(!closes(&ui), "not before it is answered");
    ui.press("palette > Cancel");
    assert!(!closes(&ui), "Cancel stays");
    press(&mut ui, "Quit");
    assert_eq!(ui.labels("palette"), question("closing", "this scene"));
    ui.press("palette > Don't save");
    assert!(closes(&ui), "Don't save quits");
    let mut ui = Ui::new();
    press(&mut ui, "Quit");
    assert!(closes(&ui), "with nothing to lose it quits at once");
}

/// A window with two parts, where `key` deletes the active part, asking nothing, as Part >
/// Delete > This part does, and the part after it becomes active; one undo brings it back.
fn deletes_the_active_part(key: &str) {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    let active = ui.state()["parts.active"].clone();
    ui.tap(key, Modifiers::default());
    assert_eq!(parts(&mut ui), 1, "{key}: the active part goes");
    assert_ne!(ui.state()["parts.active"], active, "another is active");
    assert!(ui.labels("palette").is_empty(), "asking nothing");
    ui.tap("Z", held("Ctrl"));
    assert_eq!(parts(&mut ui), 2, "one undo brings it back");
}

/// Review 0009, R12: Delete deletes the active part.
#[test]
fn delete_deletes_the_active_part() {
    deletes_the_active_part("Delete");
}

/// A Mac laptop has no Delete key: its delete key sends Backspace (Apple's Mac keyboard
/// shortcuts), which deletes the active part as Delete does, as Blender's Industry Compatible
/// keymap binds both keys to delete (UI-keys, 2026-10-14).
#[test]
fn backspace_the_mac_laptops_delete_key_deletes_the_active_part() {
    deletes_the_active_part("Backspace");
}

/// A text field keeps Backspace: in the paint hex field it takes back the last letter typed,
/// and no part goes.
#[test]
fn backspace_in_a_text_field_takes_back_a_letter_and_deletes_no_part() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    type_hex(&mut ui, "3366CCD", "Backspace");
    assert_eq!(parts(&mut ui), 2, "no part goes");
    ui.tap("Return", Modifiers::default());
    assert_eq!(
        ui.state()["color.main"],
        json!("#3366cc"),
        "D is taken back"
    );
}
