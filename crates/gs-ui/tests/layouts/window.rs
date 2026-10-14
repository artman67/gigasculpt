//! The window's part in scene files (decision 0016, section 17, amended 2026-10-13), as the
//! window hands it to the interface: its title, closing it with Save, Don't save or Cancel when
//! there are unsaved changes, a file dropped on it, and a file named on the command line. The
//! close and the drop come in as the window's input; the title is what the window shows.

use gs_platform::{Answer, Input, Modifiers, Pick};
use gs_protocol::Step;
use serde_json::json;

use crate::rig::{Ui, folder};
use crate::scene::{answer, form, parts, problem, question};

/// The window asked to close, as its close button or the system asks.
fn close(ui: &mut Ui) {
    ui.input(&[Input::Close], Modifiers::default());
}

/// Whether the window closes now.
fn closes(ui: &Ui) -> bool {
    ui.host.lock().shell.closes()
}

/// A file dropped on the window.
fn drop_file(ui: &mut Ui, path: &std::path::Path) {
    ui.input(&[Input::Dropped(path.to_path_buf())], Modifiers::default());
}

/// The window's title.
fn title(ui: &mut Ui) -> String {
    ui.settle();
    ui.host.lock().shell.title()
}

/// Whether the scene has changes no save holds (saves are not logged, so tests read this).
fn unsaved(ui: &mut Ui) -> bool {
    ui.state()["file.unsaved"] == true
}

/// The title names the scene's file, `*` first while it has unsaved changes; a new scene with
/// nothing in it is the program's name alone, and once it has something it is Untitled.
#[test]
fn the_title_names_the_scene_and_marks_unsaved_changes() {
    let mut ui = Ui::new();
    let dir = folder("window-title");
    assert_eq!(title(&mut ui), "GigaSculpt");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    assert_eq!(title(&mut ui), "*Untitled - GigaSculpt");
    ui.ok("file.save", json!({ "path": dir.join("head.gss") }));
    assert_eq!(title(&mut ui), "head.gss - GigaSculpt");
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    assert_eq!(title(&mut ui), "*head.gss - GigaSculpt");
}

/// With nothing to lose, the window closes at once, asking nothing.
#[test]
fn closing_with_nothing_to_lose_closes_at_once() {
    let mut ui = Ui::new();
    close(&mut ui);
    assert!(closes(&ui), "a new empty scene");
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok(
        "file.save",
        json!({ "path": folder("close-saved").join("a.gss") }),
    );
    close(&mut ui);
    assert!(closes(&ui), "a saved scene");
    assert!(ui.labels("palette").is_empty(), "asking nothing");
}

/// Closing with unsaved changes asks Save, Don't save or Cancel: Cancel and Escape keep the
/// window open with nothing saved, Don't save closes it letting the changes go.
#[test]
fn closing_with_unsaved_changes_asks_and_cancel_or_dont_save_do_what_they_say() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    close(&mut ui);
    assert_eq!(ui.labels("palette"), question("closing", "this scene"));
    assert!(!closes(&ui), "not before it is answered");
    ui.press("palette > Cancel");
    assert!(ui.labels("palette").is_empty(), "Cancel shuts the question");
    close(&mut ui);
    ui.tap("Escape", Modifiers::default());
    assert!(ui.labels("palette").is_empty(), "and so does Escape");
    assert!(!closes(&ui), "the window stays");
    assert_eq!((unsaved(&mut ui), parts(&mut ui)), (true, 1), "nothing ran");
    close(&mut ui);
    ui.press("palette > Don't save");
    assert!(closes(&ui), "Don't save closes it");
    assert!(unsaved(&mut ui), "with nothing saved");
}

/// Save on the close question writes the scene's own file, then the window closes; Enter
/// answers Save.
#[test]
fn save_on_closing_saves_the_scene_then_closes() {
    for answer_it in ["press", "Enter"] {
        let dir = folder(&format!("close-save-{answer_it}"));
        let mut ui = Ui::new();
        ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
        ui.ok("file.save", json!({ "path": dir.join("a.gss") }));
        ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
        close(&mut ui);
        assert_eq!(ui.labels("palette"), question("closing", "a.gss"));
        match answer_it {
            "press" => ui.press("palette > Save"),
            _ => ui.tap("Enter", Modifiers::default()),
        }
        assert!(!unsaved(&mut ui), "{answer_it}: saved: {:?}", problem(&ui));
        assert!(
            dir.join("a.backup.gss").is_file(),
            "{answer_it}: into a.gss"
        );
        assert!(closes(&ui), "{answer_it}: closes once saved");
    }
}

/// Save on the close question for a scene with no file asks where first, in the form where no
/// system dialog shows; the window closes only once the save lands.
#[test]
fn save_on_closing_a_scene_with_no_file_asks_where_then_closes() {
    let dir = folder("close-save-as");
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    close(&mut ui);
    ui.press("palette > Save");
    let asked = form(&ui).map(|f| (f.0, f.2.into_iter().map(|(n, _)| n).collect()));
    assert_eq!(asked, Some(("file.save".into(), vec!["path".to_string()])));
    assert!(!closes(&ui), "not before the save lands");
    answer(&mut ui, &dir.join("first").display().to_string());
    assert!(dir.join("first.gss").is_file(), "saved: {:?}", problem(&ui));
    assert!(closes(&ui), "then it closes");
}

/// The same with the system's Save dialog: the window closes once the file picked is saved,
/// and stays open when the dialog is cancelled.
#[test]
fn save_on_closing_through_the_save_dialog_closes_once_saved_or_stays_on_cancel() {
    let dir = folder("close-dialog");
    let mut ui = Ui::new();
    ui.host.lock().shell.palette.dialogs = true;
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    for (pick, closed) in [
        (Answer::Cancelled, false),
        (Answer::Chosen(dir.join("b.gss")), true),
    ] {
        close(&mut ui);
        ui.press("palette > Save");
        let shown = ui.host.lock().shell.take_dialog().map(|d| d.pick);
        assert_eq!(shown, Some(Pick::Save), "Save asks where");
        assert!(!closes(&ui), "not yet");
        ui.input(&[Input::Dialog(pick)], Modifiers::default());
        assert_eq!(closes(&ui), closed, "{:?}", problem(&ui));
    }
    assert!(dir.join("b.gss").is_file());
}

/// A scene file dropped on the window opens, asking first about unsaved changes; a mesh file
/// is imported into the scene; any other file is refused, the status line saying why.
#[test]
fn a_dropped_scene_opens_asking_first_and_a_dropped_mesh_is_imported() {
    let mut ui = Ui::new();
    let dir = folder("window-drop");
    let (scene, mesh, notes) = (
        dir.join("a.gss"),
        dir.join("tri.obj"),
        dir.join("notes.txt"),
    );
    std::fs::write(&notes, "a note").unwrap_or_else(|e| panic!("{e}"));
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.export", json!({ "path": mesh }));
    ui.ok("file.save", json!({ "path": scene }));
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    drop_file(&mut ui, &scene);
    let before = "opening another scene";
    assert_eq!(ui.labels("palette"), question(before, "a.gss"));
    assert_eq!(parts(&mut ui), 2, "nothing dropped while it asks");
    ui.press("palette > Don't save");
    assert_eq!(parts(&mut ui), 1, "a.gss opened: {:?}", problem(&ui));
    drop_file(&mut ui, &mesh);
    assert_eq!(parts(&mut ui), 2, "tri.obj imported: {:?}", problem(&ui));
    drop_file(&mut ui, &notes);
    let said = problem(&ui).unwrap_or_default();
    assert!(said.contains("is not a scene"), "{said}");
    assert_eq!(parts(&mut ui), 2, "nothing ran");
    ui.ok("file.save", json!({ "path": dir.join("b.gss") }));
    drop_file(&mut ui, &scene);
    assert!(
        ui.labels("palette").is_empty(),
        "nothing to lose: no question"
    );
    assert_eq!(parts(&mut ui), 1, "a.gss opened at once");
}

/// `gigasculpt PATH`: the file named on the command line opens as the window starts, in the
/// place of the last scene the Open last setting would open.
#[test]
fn a_file_named_on_the_command_line_opens_in_the_place_of_the_last_scene() {
    let dir = folder("command-line-open");
    let state = dir.join("state");
    let (named, last) = (dir.join("named.gss"), dir.join("last.gss"));
    let mut ui = Ui::launch(&dir, &state);
    ui.settle();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": named }));
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    ui.ok("file.save", json!({ "path": last }));
    ui.ok("ui.settings", json!({ "open": true, "page": "Files" }));
    ui.settle();
    ui.scroll_to("settings", "Open last scene");
    ui.press("settings > Open last scene");
    assert_eq!(ui.state()["settings.open_last"], json!(true), "switched on");
    ui.session.engine.close();
    drop(ui);
    let mut ui = Ui::launch(&dir, &state);
    let step = Step {
        command: "file.open".into(),
        args: json!({ "path": named }),
    };
    ui.host.lock().shell.open_at_start(step);
    ui.settle();
    let path = named.display().to_string();
    assert_eq!(ui.state()["file.path"], json!(path), "{:?}", problem(&ui));
    assert_eq!(
        ui.logged("file.open").len(),
        1,
        "only the file named opened"
    );
    assert_eq!(parts(&mut ui), 1);
}
