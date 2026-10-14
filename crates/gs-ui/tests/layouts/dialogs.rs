//! The system's file dialogs (decision 0016, section 17, amended 2026-10-13), through the seam
//! the window shows them by: the dialog the interface asks for (which kind, its filters, the
//! folder it starts in and the name it suggests), and its answer handed back as the window
//! hands it, as input. Each action is asked for the way a person asks: its key or its menu
//! row. Whether SDL shows the dialog on screen is the window's part (gs-platform's tests and a
//! run by hand); these tests stand in for the person's pick.

use std::path::{Path, PathBuf};

use gs_platform::{Answer, Dialog, Filter, Input, Modifiers, Pick};
use serde_json::{Value, json};

use crate::header::open;
use crate::rig::{Ui, center, folder, held};
use crate::scene::{form, parts, press, problem};

/// A window that shows the system's dialogs, as the real one does.
fn with_dialogs() -> Ui {
    let ui = Ui::new();
    ui.host.lock().shell.palette.dialogs = true;
    ui
}

/// The dialog the interface asked the window to show since the last look, if any.
fn shown(ui: &Ui) -> Option<Dialog> {
    ui.host.lock().shell.take_dialog()
}

/// The dialog's answer, as the window hands it to the interface.
fn answer(ui: &mut Ui, answer: Answer) {
    ui.input(&[Input::Dialog(answer)], Modifiers::default());
}

/// A filter offered.
fn filter(name: &str, pattern: &str) -> Filter {
    Filter {
        name: name.into(),
        pattern: pattern.into(),
    }
}

fn scenes() -> Vec<Filter> {
    vec![filter("GigaSculpt scene", "gss"), filter("All files", "*")]
}

fn own(ui: &mut Ui) -> Value {
    ui.state()["file.path"].clone()
}

fn text(path: &Path) -> Value {
    json!(path.display().to_string())
}

fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| panic!("a home folder"))
}

/// Ctrl+Shift+S asks the Save dialog for a scene file, then every file, in the scene's folder
/// and suggesting its file's name; the file picked is saved to, with `.gss` added when its
/// name has no extension.
#[test]
fn save_as_asks_the_save_dialog_in_the_scenes_folder() {
    let mut ui = with_dialogs();
    let dir = folder("dialog-save-as");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": dir.join("a.gss") }));
    ui.tap("S", held_two("Ctrl", "Shift"));
    let asked = Dialog {
        pick: Pick::Save,
        filters: scenes(),
        folder: Some(dir.clone()),
        name: Some("a.gss".into()),
    };
    assert_eq!(shown(&ui), Some(asked));
    assert!(form(&ui).is_none(), "no form while the dialog asks");
    answer(&mut ui, Answer::Chosen(dir.join("b")));
    assert_eq!(own(&mut ui), text(&dir.join("b.gss")), "{:?}", problem(&ui));
    assert!(dir.join("b.gss").is_file(), "written");
}

/// Ctrl+S on a scene with no file asks the Save dialog, suggesting `Untitled.gss` in the home
/// folder, with no problem shown; Cancel runs nothing.
#[test]
fn a_first_ctrl_s_asks_the_save_dialog_and_cancel_runs_nothing() {
    let mut ui = with_dialogs();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.tap("S", held("Ctrl"));
    let asked = Dialog {
        pick: Pick::Save,
        filters: scenes(),
        folder: Some(home()),
        name: Some("Untitled.gss".into()),
    };
    assert_eq!(shown(&ui), Some(asked));
    assert_eq!(problem(&ui), None, "asking is not a failure");
    answer(&mut ui, Answer::Cancelled);
    assert_eq!(own(&mut ui), Value::Null, "nothing saved");
    assert!(form(&ui).is_none(), "and nothing asked");
    assert_eq!(shown(&ui), None);
}

/// While one dialog waits for its answer, another is not shown: the status line says why.
#[test]
fn one_dialog_at_a_time() {
    let mut ui = with_dialogs();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.tap("S", held_two("Ctrl", "Shift"));
    assert!(shown(&ui).is_some());
    ui.tap("S", held_two("Ctrl", "Shift"));
    assert_eq!(shown(&ui), None, "no second dialog");
    let said = problem(&ui).unwrap_or_default();
    assert!(said.contains("dialog is open"), "{said}");
}

/// A dialog that cannot show asks in the palette's form instead, and the status line says why;
/// later saves go to the form at once, without saying it again.
#[test]
fn a_dialog_that_cannot_show_asks_in_the_form_and_says_why_once() {
    let mut ui = with_dialogs();
    let dir = folder("dialog-failed");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": dir.join("a.gss") }));
    ui.tap("S", held_two("Ctrl", "Shift"));
    assert!(shown(&ui).is_some());
    answer(&mut ui, Answer::Failed("no file dialog driver".into()));
    let asked = form(&ui).map(|f| (f.0, f.2.into_iter().map(|(n, _)| n).collect()));
    assert_eq!(asked, Some(("file.save".into(), vec!["path".to_string()])));
    let said = problem(&ui).unwrap_or_default();
    assert!(said.contains("no file dialog driver"), "{said}");
    crate::scene::answer(&mut ui, "b.gss");
    assert_eq!(own(&mut ui), text(&dir.join("b.gss")), "{:?}", problem(&ui));
    ui.host.lock().shell.status.problem = None;
    ui.tap("S", held_two("Ctrl", "Shift"));
    assert_eq!(shown(&ui), None, "the form at once");
    assert!(form(&ui).is_some());
    assert_eq!(problem(&ui), None, "said once");
}

/// Ctrl+S held with another key: Ctrl and Shift.
fn held_two(a: &str, b: &str) -> Modifiers {
    let (a, b) = (held(a), held(b));
    Modifiers {
        shift: a.shift || b.shift,
        ctrl: a.ctrl || b.ctrl,
        alt: a.alt || b.alt,
        logo: false,
    }
}

/// The dialog's kind, its filters' patterns and the name it suggests.
fn kind(dialog: Option<Dialog>) -> Option<(Pick, Vec<String>, Option<String>)> {
    let d = dialog?;
    let patterns = d.filters.into_iter().map(|f| f.pattern).collect();
    Some((d.pick, patterns, d.name))
}

/// Answers the dialog asked for with Cancel: nothing runs, and nothing else asks.
fn cancel(ui: &mut Ui) {
    answer(ui, Answer::Cancelled);
    assert!(form(ui).is_none(), "Cancel asks nothing");
    assert_eq!(shown(ui), None, "Cancel shows nothing else");
}

/// Open, Import, Export, Save Part file, Save next Part file the first time, Save brush, Load
/// brush and Reference from a file each ask their own dialog, offering their files first and
/// every file last; Cancel on each runs nothing.
#[test]
fn every_file_action_asks_its_own_dialog() {
    let mut ui = with_dialogs();
    let dir = folder("dialog-actions");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": dir.join("head.gss") }));
    let meshes = || vec!["obj;ply;stl;ztl;zpr".to_string(), "*".into()];
    let gss = || vec!["gss".to_string(), "*".into()];
    ui.tap("O", held("Ctrl"));
    assert_eq!(kind(shown(&ui)), Some((Pick::Open, gss(), None)), "Open");
    cancel(&mut ui);
    press(&mut ui, "Import…");
    assert_eq!(
        kind(shown(&ui)),
        Some((Pick::Open, meshes(), None)),
        "Import"
    );
    cancel(&mut ui);
    press(&mut ui, "Export…");
    let out = vec!["obj;ply;stl".to_string(), "*".into()];
    let named = Some("head.obj".to_string());
    assert_eq!(kind(shown(&ui)), Some((Pick::Save, out, named)), "Export");
    cancel(&mut ui);
    let part = ui.host.lock().shell.state.active.clone();
    let part = part.map(|p| format!("{p}.gss"));
    ui.tap("T", held_two("Ctrl", "Shift"));
    assert_eq!(
        kind(shown(&ui)),
        Some((Pick::Save, gss(), part.clone())),
        "Part"
    );
    cancel(&mut ui);
    // File lists the Part's Save next as "Save next" under Part file, after the scene's.
    open(&mut ui, "File");
    let rows = crate::header::items(&mut ui);
    let next = rows
        .iter()
        .find(|w| w["label"] == "Save next" && w["command"]["args"]["parts"].is_array());
    let [x, y] = center(
        &next.unwrap_or_else(|| panic!("File lists the Part's Save next: {rows:?}"))["rect"],
    );
    ui.tap_at(x, y);
    ui.settle();
    assert_eq!(
        kind(shown(&ui)),
        Some((Pick::Save, gss(), part)),
        "Save next Part"
    );
    cancel(&mut ui);
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let toml = || vec!["toml".to_string(), "*".into()];
    for (pick, row) in [(Pick::Save, "Save brush…"), (Pick::Open, "Load brush…")] {
        ui.press("brush > ···");
        ui.press(&format!("brush > {row}"));
        let asked = kind(shown(&ui)).map(|(p, f, _)| (p, f));
        assert_eq!(asked, Some((pick, toml())), "{row}");
        cancel(&mut ui);
    }
    open(&mut ui, "View");
    ui.press("menu > From a file…");
    assert_eq!(
        kind(shown(&ui)),
        Some((Pick::Open, meshes(), None)),
        "Reference"
    );
    cancel(&mut ui);
    assert_eq!(parts(&mut ui), 1, "nothing ran");
}

/// The file commands a person reaches only from the palette's list ask a dialog too: Save
/// floor preset the Save dialog for a TOML file, suggesting Floor.toml, and the materials folder
/// the folder dialog; the file picked is written.
#[test]
fn the_palettes_own_file_commands_ask_their_dialog_too() {
    let mut ui = with_dialogs();
    let dir = folder("dialog-palette");
    let pick = |ui: &mut Ui, command: &str| {
        ui.ok("ui.palette", json!({ "query": command }));
        ui.tap("Enter", Modifiers::default());
        kind(shown(ui))
    };
    let toml = vec!["toml".to_string(), "*".into()];
    let floor = Some((Pick::Save, toml, Some("Floor.toml".to_string())));
    assert_eq!(pick(&mut ui, "view.floor_save"), floor, "Save floor preset");
    answer(&mut ui, Answer::Chosen(dir.join("floor")));
    assert!(dir.join("floor.toml").is_file(), "{:?}", problem(&ui));
    let folders = Some((Pick::Folder, Vec::new(), None));
    assert_eq!(
        pick(&mut ui, "material.folder"),
        folders,
        "the materials folder"
    );
    cancel(&mut ui);
}

/// A file the Save dialog picked that exists was already asked about by the dialog: it is
/// replaced with no second question. Open picking a mesh file imports it, logged as any step.
#[test]
fn a_file_the_dialog_picked_runs_without_asking_again() {
    let mut ui = with_dialogs();
    let dir = folder("dialog-replace");
    let (a, b) = (dir.join("a.gss"), dir.join("b.gss"));
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.export", json!({ "path": dir.join("tri.obj") }));
    ui.ok("file.save", json!({ "path": a }));
    std::fs::write(&b, "kept").unwrap_or_else(|e| panic!("{e}"));
    ui.tap("S", held_two("Ctrl", "Shift"));
    assert!(shown(&ui).is_some());
    answer(&mut ui, Answer::Chosen(b.clone()));
    assert!(form(&ui).is_none(), "no second question");
    assert_eq!(own(&mut ui), text(&b), "{:?}", problem(&ui));
    ui.tap("O", held("Ctrl"));
    assert!(shown(&ui).is_some());
    answer(&mut ui, Answer::Chosen(dir.join("tri.obj")));
    assert_eq!(parts(&mut ui), 2, "imported: {:?}", problem(&ui));
    let imports = ui.logged("file.import");
    let roles: Vec<_> = imports.iter().map(|s| s["role"].clone()).collect();
    assert_eq!(roles, [json!("human")], "logged as any step: {imports:?}");
}

/// A name the Save dialog picked without an extension gets one, and the dialog never asked
/// about the file of that name: one that exists is asked about before it is replaced.
#[test]
fn a_picked_name_given_its_extension_asks_before_it_replaces() {
    let mut ui = with_dialogs();
    let dir = folder("dialog-extension-replace");
    let (plain, scene) = (dir.join("c"), dir.join("c.gss"));
    std::fs::write(&plain, "plain").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(&scene, "kept").unwrap_or_else(|e| panic!("{e}"));
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.tap("S", held_two("Ctrl", "Shift"));
    assert!(shown(&ui).is_some(), "Save as asks the dialog");
    answer(&mut ui, Answer::Chosen(plain));
    let asks = ["c.gss exists. Replace it?", "Replace", "Cancel"];
    assert_eq!(ui.labels("palette"), asks, "{:?}", problem(&ui));
    let kept = std::fs::read_to_string(&scene).ok();
    assert_eq!(kept.as_deref(), Some("kept"), "not replaced unasked");
}
