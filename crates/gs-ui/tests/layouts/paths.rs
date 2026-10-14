//! A path typed into a file form is the path that runs (decision 0016, section 17, amended
//! 2026-10-13): a full path typed after the folder the field starts with replaces that folder,
//! quotes a file manager copies round it go, `~` is the home folder, a folder that is gone is
//! not offered, and a refusal quotes the path the engine got, so a stray prefix or space shows.

use gs_platform::Modifiers;
use serde_json::json;

use crate::rig::{Ui, folder, held};
use crate::scene::{answer, form, parts, press};

/// The folder a field starts in for `dir`: its path and a separator.
fn in_folder(dir: &std::path::Path) -> String {
    format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR)
}

/// What the open form's first field starts with.
fn start(ui: &Ui) -> Option<String> {
    form(ui).map(|f| f.2[0].1.clone())
}

/// What the status line says is wrong; nothing, empty.
fn problem(ui: &Ui) -> String {
    crate::scene::problem(ui).unwrap_or_default()
}

/// The scene's own file, as the engine reports it.
fn own(ui: &mut Ui) -> serde_json::Value {
    ui.state()["file.path"].clone()
}

/// A person types a full path after the folder Save scene as starts in, into a folder whose
/// name has a space: the scene is saved there, at exactly that path. Open does the same, and a
/// path pasted with the quotes a file manager puts round it opens too.
#[test]
fn a_full_path_typed_after_the_folder_a_field_starts_in_is_that_path() {
    let mut ui = Ui::new();
    let dir = folder("full-path");
    let (a, spaced) = (dir.join("a"), dir.join("My Art").join("Oct"));
    for made in [&a, &spaced] {
        std::fs::create_dir_all(made).unwrap_or_else(|e| panic!("{e}"));
    }
    let one = a.join("one.gss");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": one }));
    press(&mut ui, "Save as…");
    assert_eq!(
        start(&ui),
        Some(in_folder(&a)),
        "starts in the scene's folder"
    );
    let typed = spaced.join("gigachad.gss");
    answer(&mut ui, &typed.display().to_string());
    assert!(form(&ui).is_none(), "it ran: {}", problem(&ui));
    assert_eq!(
        own(&mut ui),
        json!(typed.display().to_string()),
        "exactly that path"
    );
    assert!(typed.is_file(), "written where it was typed");
    press(&mut ui, "Open…");
    assert_eq!(start(&ui), Some(in_folder(&spaced)));
    answer(&mut ui, &one.display().to_string());
    assert_eq!(
        own(&mut ui),
        json!(one.display().to_string()),
        "{}",
        problem(&ui)
    );
    press(&mut ui, "Open…");
    answer(&mut ui, &format!("'{}'", typed.display()));
    assert_eq!(
        own(&mut ui),
        json!(typed.display().to_string()),
        "{}",
        problem(&ui)
    );
}

/// Save scene as and Import start in no folder once the folder they would start in is gone,
/// so what is typed is the whole path.
#[test]
fn a_folder_that_is_gone_is_not_offered() {
    let mut ui = Ui::new();
    let dir = folder("gone-folder");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap_or_else(|e| panic!("{e}"));
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": sub.join("one.gss") }));
    press(&mut ui, "Save as…");
    assert_eq!(start(&ui), Some(in_folder(&sub)), "there while it is there");
    ui.ok("ui.palette", json!({ "open": false }));
    std::fs::remove_dir_all(&sub).unwrap_or_else(|e| panic!("{e}"));
    press(&mut ui, "Save as…");
    assert_eq!(
        start(&ui),
        Some(String::new()),
        "Save scene as: a gone folder is not offered"
    );
    ui.ok("ui.palette", json!({ "open": false }));
    press(&mut ui, "Import…");
    assert_eq!(start(&ui), Some(String::new()), "Import: nor here");
}

/// A save into a folder that does not exist is refused, and the form, kept open, quotes the
/// folder the engine got under its field, its space and all; `~` typed first is the home folder.
#[test]
fn a_refusal_quotes_the_path_the_engine_got() {
    let mut ui = Ui::new();
    let dir = folder("quoted");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    press(&mut ui, "Save as…");
    assert_eq!(
        start(&ui),
        Some(String::new()),
        "a new scene starts in no folder"
    );
    let gone = dir.join("no such ");
    answer(&mut ui, &gone.join("head.gss").display().to_string());
    let quoted = format!("\"{}\"", gone.display());
    let said = palette(&mut ui).join(" | ");
    assert!(said.contains(&quoted), "{quoted} in: {said}");
    ui.tap("Escape", Modifiers::default());
    let away = format!("gigasculpt-no-such-folder-{}", std::process::id());
    let home = std::env::home_dir().unwrap_or_else(|| panic!("a home folder"));
    press(&mut ui, "Save as…");
    answer(&mut ui, &format!("~/{away}/head.gss"));
    let quoted = format!("\"{}\"", home.join(&away).display());
    let said = palette(&mut ui).join(" | ");
    assert!(said.contains(&quoted), "{quoted} in: {said}");
}

/// The palette's labels: a question's line and its buttons, or a form's.
fn palette(ui: &mut Ui) -> Vec<String> {
    ui.labels("palette")
}

/// Save scene as onto a file that exists asks to replace it, with no problem shown: Cancel
/// leaves the file alone, Replace writes over it. Export onto a file that exists asks the
/// same (review 0009, B2).
#[test]
fn save_as_or_export_onto_a_file_that_exists_asks_to_replace_it() {
    let mut ui = Ui::new();
    let dir = folder("replace");
    let (a, b) = (dir.join("a.gss"), dir.join("b.gss"));
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": a }));
    std::fs::write(&b, "kept").unwrap_or_else(|e| panic!("{e}"));
    let asks = ["b.gss exists. Replace it?", "Replace", "Cancel"];
    press(&mut ui, "Save as…");
    answer(&mut ui, "b.gss");
    assert_eq!(palette(&mut ui), asks, "asks before it replaces");
    assert_eq!(problem(&ui), "", "asking is not a failure");
    ui.press("palette > Cancel");
    assert_eq!(std::fs::read_to_string(&b).ok().as_deref(), Some("kept"));
    press(&mut ui, "Save as…");
    answer(&mut ui, "b.gss");
    ui.press("palette > Replace");
    assert_eq!(
        own(&mut ui),
        json!(b.display().to_string()),
        "{}",
        problem(&ui)
    );
    let out = dir.join("out.obj");
    std::fs::write(&out, "kept").unwrap_or_else(|e| panic!("{e}"));
    press(&mut ui, "Export…");
    answer(&mut ui, "out.obj");
    let asks = ["out.obj exists. Replace it?", "Replace", "Cancel"];
    assert_eq!(palette(&mut ui), asks, "Export asks the same");
    ui.press("palette > Replace");
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    assert!(
        text.starts_with("v ") || text.contains("\nv "),
        "replaced: {}",
        problem(&ui)
    );
}

/// Ctrl+S on a scene first saved in an older format, which now needs the new one, asks with
/// no problem shown: Save as… asks for another file, Cancel leaves the file alone, and Save
/// writes it in the new format (review 0009, B3).
#[test]
fn saving_a_scene_from_an_older_format_asks_to_save_it_in_the_new_one() {
    let mut ui = Ui::new();
    let dir = folder("upgrade");
    let old = dir.join("old.gss");
    let golden = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../gs-engine/tests/scene/golden/v1.gss"
    );
    std::fs::copy(golden, &old).unwrap_or_else(|e| panic!("{e}"));
    ui.ok("file.open", json!({ "path": old }));
    ui.ok("level.subdivide", json!({ "part": "painted" }));
    let before = std::fs::read(&old).unwrap_or_else(|e| panic!("{e}"));
    ui.tap("S", held("Ctrl"));
    let asks = palette(&mut ui);
    assert_eq!(asks[1..], ["Save", "Save as…", "Cancel"], "{asks:?}");
    assert!(asks[0].contains("old.gss"), "names the file: {}", asks[0]);
    assert_eq!(problem(&ui), "", "asking is not a failure");
    ui.press("palette > Save as…");
    let asked = form(&ui).map(|f| (f.0, f.2.into_iter().map(|(n, _)| n).collect()));
    assert_eq!(asked, Some(("file.save".into(), vec!["path".to_string()])));
    ui.ok("ui.palette", json!({ "open": false }));
    // A frame lets go of the form's field, as the window's next frame does.
    ui.settle();
    ui.tap("S", held("Ctrl"));
    ui.press("palette > Cancel");
    let after = std::fs::read(&old).unwrap_or_else(|e| panic!("{e}"));
    assert!(after == before, "the file is untouched");
    ui.tap("S", held("Ctrl"));
    ui.press("palette > Save");
    let seen = ui.ok("file.inspect", json!({ "path": old }));
    assert_eq!(seen["oldest_reader"], json!(2), "{}", problem(&ui));
}

/// A name typed with no extension gets the one the form writes: `.gss` for Save scene as,
/// `.obj` for Export (review 0009, C6).
#[test]
fn a_name_typed_without_an_extension_gets_one() {
    let mut ui = Ui::new();
    let dir = folder("extension");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": dir.join("a.gss") }));
    press(&mut ui, "Save as…");
    answer(&mut ui, "head");
    let head = dir.join("head.gss");
    assert_eq!(
        own(&mut ui),
        json!(head.display().to_string()),
        "{}",
        problem(&ui)
    );
    press(&mut ui, "Export…");
    answer(&mut ui, "out");
    assert!(dir.join("out.obj").is_file(), "{}", problem(&ui));
}

/// Open given a mesh file imports it into the scene instead of refusing it as no scene
/// (review 0009, C7).
#[test]
fn open_on_a_mesh_file_imports_it() {
    let mut ui = Ui::new();
    let dir = folder("open-mesh");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.export", json!({ "path": dir.join("tri.obj") }));
    ui.ok("file.save", json!({ "path": dir.join("a.gss") }));
    press(&mut ui, "Open…");
    answer(&mut ui, "tri.obj");
    assert_eq!(parts(&mut ui), 2, "imported: {}", problem(&ui));
    assert_eq!(ui.logged("file.import").len(), 1, "as Import");
}
