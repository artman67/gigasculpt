//! A refusal reads as a person's sentence wherever it comes from (decision 0025, section 2,
//! amended 2026-10-14 (UI-words); (Rough-tray-notes) (1)): a brush preset or a reference given
//! only a folder asks for a file name, as Open does, in place of the system's "Is a directory",
//! the form kept open; Open and Export refused keep their form as typed, as Save does. Each is
//! taken as a person takes it, from a menu and its form. Since UI-last (2026-10-14) a save
//! into a folder that is not there says so in place of the system's "No such file or
//! directory", Load curve given a folder asks for a file name, and a gizmo drag refused as it
//! ends reads as a refused call does.

use std::path::MAIN_SEPARATOR;

use gs_platform::Modifiers;
use serde_json::json;

use crate::header::open;
use crate::refusals::{said, typed};
use crate::rig::{Ui, folder};
use crate::scene::{answer, form, press, problem};

/// Runs the open form with `typing` after what it starts with, which then names only a folder,
/// and checks what a person reads: a file name asked for, no system words, the form still open.
fn folder_only(ui: &mut Ui, what: &str, typing: &str) {
    let start = typed(ui).unwrap_or_else(|| panic!("{what} asks for the file"));
    assert!(
        format!("{start}{typing}").ends_with(MAIN_SEPARATOR),
        "{what}: {start:?}"
    );
    answer(ui, typing);
    let words = said(ui);
    assert!(words.contains("Type a file name"), "{what}: {words}");
    for system in ["os error", "Is a directory", "E_"] {
        assert!(!words.contains(system), "{what}: {words}");
    }
    assert!(form(ui).is_some(), "{what}: the form stays open");
    ui.tap("Escape", Modifiers::default());
}

/// Load brush… and Save brush…, once a brush was saved, start in its folder; run so, each asks
/// for a file name.
#[test]
fn a_brush_file_form_given_a_folder_asks_for_a_file_name() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let dir = folder("brush-folder-only");
    ui.press("brush > ···");
    ui.press("brush > Save brush…");
    answer(&mut ui, &dir.join("draw.toml").display().to_string());
    assert!(dir.join("draw.toml").is_file(), "saved: {}", said(&mut ui));
    for pick in ["Load brush…", "Save brush…"] {
        ui.press("brush > ···");
        ui.press(&format!("brush > {pick}"));
        folder_only(&mut ui, pick, "");
    }
}

/// View > From a file… given only a folder asks for a file name.
#[test]
fn a_reference_given_a_folder_asks_for_a_file_name() {
    let mut ui = Ui::new();
    let dir = folder("reference-folder-only");
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    open(&mut ui, "View");
    ui.press("menu > From a file…");
    let typing = format!("{}{MAIN_SEPARATOR}", dir.display());
    folder_only(&mut ui, "From a file…", &typing);
}

/// Refusals-notes (4): Open and Export refused keep their form as typed, by the path Save's
/// takes (`refusals::save_as_a_mesh_name_points_to_export`): Open given a file that is gone,
/// and Export into a folder that is gone, each say why under the field, naming the file.
#[test]
fn open_and_export_refused_keep_their_form_as_typed() {
    let dir = folder("refused-kept-as-typed");
    let gone = dir.join("gone");
    let refused = [
        ("Open…", dir.join("gone.gss"), "gone.gss"),
        ("Export…", gone.join("head.obj"), "head.obj"),
    ];
    for (label, path, why) in refused {
        let mut ui = Ui::new();
        if label == "Export…" {
            ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
        }
        press(&mut ui, label);
        let start = typed(&ui).unwrap_or_else(|| panic!("{label} asks for the file"));
        answer(&mut ui, &path.display().to_string());
        let kept = format!("{start}{}", path.display());
        assert_eq!(typed(&ui), Some(kept), "{label}: the form stays, as typed");
        let words = said(&mut ui);
        assert!(
            words.contains(why) && !words.contains("E_"),
            "{label}: {words}"
        );
        assert_eq!(problem(&ui), None, "{label}: the why is under the field");
        assert!(!path.exists(), "{label}: nothing is written");
    }
}

/// What a person reads of a save into a folder that is not there: that it is not there and
/// what to do, no system words, the form still open.
fn no_folder(ui: &mut Ui, what: &str) {
    let words = said(ui);
    assert!(words.contains("make the folder first"), "{what}: {words}");
    for system in ["os error", "No such file", "E_"] {
        assert!(!words.contains(system), "{what}: {words}");
    }
    assert!(form(ui).is_some(), "{what}: the form stays open");
    ui.tap("Escape", Modifiers::default());
}

/// Export… and Save brush… into a folder that is not there say so, as Save does, in place of
/// the system's "No such file or directory"; so does a curve's save, which the window finds
/// by the same words.
#[test]
fn a_save_into_a_folder_that_is_not_there_says_so() {
    let mut ui = Ui::new();
    let gone = folder("refused-no-folder").join("gone");
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    press(&mut ui, "Export…");
    answer(&mut ui, &gone.join("head.obj").display().to_string());
    no_folder(&mut ui, "Export…");
    ui.press("brush > ···");
    ui.press("brush > Save brush…");
    answer(&mut ui, &gone.join("draw.toml").display().to_string());
    no_folder(&mut ui, "Save brush…");
    let curve = json!({ "path": gone.join("soft.toml"), "curve": [[0, 0], [1, 1]] });
    let (_, message) = ui.err("curve.save", curve);
    assert!(message.contains("no folder"), "curve.save: {message}");
    assert!(!gone.exists(), "nothing is written");
}

/// Load curve, from the palette's list, given only a folder asks for a file name, as Load
/// brush… does, in place of the system's "Is a directory".
#[test]
fn load_curve_given_a_folder_asks_for_a_file_name() {
    let mut ui = Ui::new();
    let dir = folder("curve-folder-only");
    ui.ok("ui.palette", json!({ "query": "curve.load" }));
    ui.tap("Enter", Modifiers::default());
    let typing = format!("{}{MAIN_SEPARATOR}", dir.display());
    folder_only(&mut ui, "Load curve", &typing);
}

/// A gizmo drag the engine refuses as it ends reads as a refused call does, in a person's
/// words with what to do: here one that would carry the part so far from the scene's origin
/// that its shape would flatten. The part stays where it was.
#[test]
fn a_gizmo_drag_refused_as_it_ends_reads_as_a_sentence() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let before = ui.ok("mesh.hash", json!({}))["hash"].clone();
    let far = json!({ "op": { "move": [1.0e6, 0.0, 0.0] } });
    ui.ok("gizmo.drag", json!({ "phase": "update", "transform": far }));
    assert_ne!(
        ui.ok("mesh.hash", json!({}))["hash"],
        before,
        "the drag moves it"
    );
    ui.ok("gizmo.drag", json!({ "phase": "end" }));
    let after = ui.ok("mesh.hash", json!({}))["hash"].clone();
    assert_eq!(after, before, "refused, the part is where it was");
    let line = problem(&ui).unwrap_or_default();
    assert!(line.contains("Move it closer to the origin"), "{line}");
    for system in ["f32", "E_"] {
        assert!(!line.contains(system), "{line}");
    }
}
