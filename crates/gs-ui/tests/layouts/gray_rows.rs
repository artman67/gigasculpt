//! A row that cannot run now is gray with why before it is tapped, never live only to refuse
//! (decision 0025, section 6): one opening a form is asked about as its form's step would run
//! (review 0009, R4), and a part with detail levels, or an empty scene, grays the rows the
//! engine refuses for it ((H4-notes) (1) and (2)). Each is taken as a person meets it, from a
//! menu.

use serde_json::{Value, json};

use crate::header::{item, items, open};
use crate::rig::{Ui, folder};
use crate::scene::form;

/// Opens the header's menu `title`, then makes a call so the engine says which of its rows
/// cannot run, and draws it again.
fn judged(ui: &mut Ui, title: &str) {
    open(ui, title);
    ui.settle();
    ui.state();
    ui.settle();
}

/// The open menu's row running `command` with `args`.
fn runs(ui: &mut Ui, command: &str, args: &Value) -> Value {
    let rows = items(ui);
    let object = |v: &Value| v.as_object().cloned().unwrap_or_default();
    let found = rows.iter().find(|w| {
        let step = &w["command"];
        step["command"] == command && object(&step["args"]) == object(args)
    });
    found
        .cloned()
        .unwrap_or_else(|| panic!("a row runs {command} {args}: {rows:?}"))
}

/// Why the row `row` is gray, its hint saying the same; `None` while it runs.
fn gray(row: &Value) -> Option<String> {
    if row["enabled"] != json!(false) {
        return None;
    }
    let why = row["reason"].as_str().unwrap_or_default().to_string();
    assert_eq!(row["hint"].as_str(), Some(why.as_str()), "{row}");
    Some(why)
}

/// Review 0009, R4: File > Revert… in a scene with no file is gray, saying there is no file to
/// go back to, as Recover… is dimmed with no offer waiting, and a tap opens no form; once the
/// scene has a file it is live.
#[test]
fn revert_is_gray_while_the_scene_has_no_file() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    judged(&mut ui, "File");
    let revert = item(&mut ui, "Revert…");
    let why = gray(&revert).unwrap_or_else(|| panic!("Revert is gray: {revert}"));
    assert!(why.contains("no file to go back to"), "{why}");
    ui.press("menu > Revert…");
    assert!(form(&ui).is_none(), "no form opens: {:?}", form(&ui));
    let dir = folder("revert-gray");
    ui.ok("file.save", json!({ "path": dir.join("head.gss") }));
    judged(&mut ui, "File");
    let revert = item(&mut ui, "Revert…");
    assert_eq!(revert["enabled"], true, "with a file: {revert}");
}

/// (H4-notes) (1): in an empty scene every Delete row is gray with why, All and Hidden too,
/// which deleted nothing and said nothing.
#[test]
fn every_delete_row_is_gray_in_an_empty_scene() {
    let mut ui = Ui::new();
    judged(&mut ui, "Part");
    for scope in ["others", "hidden", "all"] {
        let row = runs(&mut ui, "part.delete", &json!({ "scope": scope }));
        let why = gray(&row).unwrap_or_else(|| panic!("Delete {scope} is gray: {row}"));
        assert!(why.contains("no parts"), "{scope}: {why}");
    }
}

/// (UI-words-notes) (1): Delete > Others with one part, and Delete > Hidden with no part
/// hidden, are gray before they are tapped, saying why, where they ran and deleted nothing.
/// With a second part Others runs, and Hidden once a part's eye is off.
#[test]
fn delete_others_and_hidden_are_gray_while_they_would_delete_nothing() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    let row = |ui: &mut Ui, scope: &str| {
        judged(ui, "Part");
        gray(&runs(ui, "part.delete", &json!({ "scope": scope })))
    };
    let others = row(&mut ui, "others");
    assert_eq!(others.as_deref(), Some("There are no other parts"));
    let hidden = row(&mut ui, "hidden");
    assert_eq!(hidden.as_deref(), Some("No part is hidden"));
    ui.ok("primitive.cube", json!({}));
    assert_eq!(row(&mut ui, "others"), None, "with a second part");
    assert_eq!(row(&mut ui, "hidden"), hidden, "both show");
    let sphere = json!({ "parts": ["sphere"], "visible": false });
    ui.ok("part.set_visible", sphere);
    assert_eq!(row(&mut ui, "hidden"), None, "the sphere's eye is off");
}

/// (UI-words-notes) (1): File > Export… in an empty scene is gray before it is tapped, saying
/// there is nothing to write, and a tap opens no form. Its form gives the path the command
/// needs, so the engine is asked about it as it would run with one; with a part it is live.
#[test]
fn export_is_gray_in_an_empty_scene() {
    let mut ui = Ui::new();
    judged(&mut ui, "File");
    let export = item(&mut ui, "Export…");
    let why = gray(&export).unwrap_or_else(|| panic!("Export is gray: {export}"));
    assert_eq!(why, "The scene has no parts to write");
    ui.press("menu > Export…");
    assert!(form(&ui).is_none(), "no form opens: {:?}", form(&ui));
    ui.ok("primitive.sphere", json!({}));
    judged(&mut ui, "File");
    let export = item(&mut ui, "Export…");
    assert_eq!(gray(&export), None, "with a part: {export}");
}

/// The same gap: Mesh > Project Detail… on a part whose lower levels are frozen is gray before
/// it is tapped, saying why, though its form still asks for the source part; unfrozen it is
/// live.
#[test]
fn project_detail_is_gray_on_a_part_with_frozen_levels() {
    let mut ui = Ui::new();
    ui.ok("primitive.cube", json!({ "subdivisions": 2 }));
    ui.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    ui.ok("level.subdivide", json!({}));
    ui.ok("level.freeze", json!({}));
    judged(&mut ui, "Mesh");
    let row = item(&mut ui, "Project Detail…");
    let why = gray(&row).unwrap_or_else(|| panic!("Project Detail is gray: {row}"));
    assert!(why.contains("frozen detail levels"), "{why}");
    ui.ok("level.unfreeze", json!({}));
    judged(&mut ui, "Mesh");
    let row = item(&mut ui, "Project Detail…");
    assert_eq!(gray(&row), None, "unfrozen: {row}");
}

/// (H4-notes) (2): with detail levels on the active part, every row whose command works on a
/// part with one level is gray before it is tapped, saying why, a row that opens a form among
/// them, and a tap runs nothing and opens no form. With one level they run.
#[test]
fn rows_that_need_one_level_are_gray_on_a_part_with_levels() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    ui.ok("level.subdivide", json!({}));
    ui.ok("primitive.cube", json!({ "subdivisions": 2 }));
    ui.ok("part.select", json!({ "part": "sphere" }));
    let rows = [
        ("Part", "part.mirror_copy", json!({ "axis": "x" })),
        ("Part", "part.split", json!({ "by": "shells" })),
        ("Part", "part.merge_down", json!({})),
        ("Mesh", "mesh.voxel_remesh", json!({})),
        ("Mesh", "mesh.symmetrize", json!({ "axis": "x" })),
    ];
    let forms = [("Mesh", "Decimate…"), ("Mesh", "Quad Remesh…")];
    let logged = |ui: &mut Ui| ["part.", "mesh."].map(|p| ui.logged(p).len());
    let before = logged(&mut ui);
    for (title, command, args) in &rows {
        judged(&mut ui, title);
        let row = runs(&mut ui, command, args);
        let why = gray(&row).unwrap_or_else(|| panic!("{command} is gray: {row}"));
        assert!(why.contains("detail levels"), "{command}: {why}");
    }
    for (title, label) in forms {
        judged(&mut ui, title);
        let row = item(&mut ui, label);
        let why = gray(&row).unwrap_or_else(|| panic!("{label} is gray: {row}"));
        assert!(why.contains("detail levels"), "{label}: {why}");
        ui.press(&format!("menu > {label}"));
        assert!(form(&ui).is_none(), "{label} opens no form");
    }
    assert_eq!(logged(&mut ui), before, "nothing ran");
    ui.ok("level.delete_lower", json!({}));
    judged(&mut ui, "Part");
    let row = runs(&mut ui, "part.split", &json!({ "by": "shells" }));
    assert_eq!(gray(&row), None, "with one level");
}
