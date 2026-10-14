//! One settings file (catalog `interface.layout-presets`, decision 0050): `ui.layout` `export`
//! writes the keymap's changes, the quick menus and the arrangement to one file, and `import`
//! reads back all of it or the parts named, leaving the others exactly as they were. A file
//! from a newer program, a part the file lacks or a key naming no command refuses the import
//! whole.

use gs_protocol::ErrorCode;
use gs_ui::Keymap;
use serde_json::{Value, json};

use crate::keymap::runs;
use crate::rig::{Ui, folder};

/// A window whose keymap changes go to `keymap.toml` in a fresh folder `name`, and the path of
/// a settings file in that folder.
fn window(name: &str) -> (Ui, std::path::PathBuf, std::path::PathBuf) {
    let dir = folder(name);
    let keys = dir.join("keymap.toml");
    let ui = Ui::new();
    ui.host.lock().shell.keymap_file = Some(keys.clone());
    (ui, keys, dir.join("settings.toml"))
}

/// The quick menus, as `ui.quick` lists them.
fn menus(ui: &mut Ui) -> Value {
    ui.ok("ui.quick", json!({}))["menus"].clone()
}

/// Catalog `interface.layout-presets`: "exporting all, changing the keymap and a quick menu,
/// then importing only the keymap leaves the quick menu changed".
#[test]
fn importing_only_the_keymap_leaves_the_quick_menu_changed() {
    let (mut ui, keys, file) = window("export-keymap");
    ui.ok(
        "ui.keymap",
        json!({ "bind": "Ctrl+K", "command": "primitive.cube" }),
    );
    let path = file.to_string_lossy().to_string();
    ui.ok("ui.layout", json!({ "export": path }));
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{e}"));
    for table in ["version = 1", "[arrangement]", "[quick]", "[[keymap.bind]]"] {
        assert!(text.contains(table), "{table} in\n{text}");
    }
    let exported = menus(&mut ui);

    // The keymap and a quick menu change, and the hand.
    let moved = json!({ "bind": "Ctrl+K", "command": "primitive.sphere", "replace": true });
    ui.ok("ui.keymap", moved);
    ui.ok(
        "ui.keymap",
        json!({ "bind": "K", "command": "mask.invert" }),
    );
    ui.ok("ui.quick", json!({ "menu": 2, "add": "history" }));
    ui.ok("ui.layout", json!({ "hand": "left" }));
    let changed = menus(&mut ui);
    assert_ne!(changed, exported);

    let only = json!({ "import": path, "parts": ["keymap"] });
    ui.ok("ui.layout", only);
    assert_eq!(runs(&ui, "Ctrl+K").as_deref(), Some("primitive.cube"));
    assert_eq!(
        runs(&ui, "K"),
        None,
        "the keymap is the file's, not added to"
    );
    assert_eq!(menus(&mut ui), changed, "the quick menu stays changed");
    assert_eq!(ui.ok("ui.layout", json!({}))["hand"], json!("left"));
    let (read, _) = Keymap::load(&keys).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(read, ui.host.lock().shell.keymap, "keymap.toml is written");

    // Everything: the quick menus and the arrangement come back too.
    ui.ok("ui.layout", json!({ "import": path }));
    assert_eq!(menus(&mut ui), exported);
    assert_eq!(ui.ok("ui.layout", json!({}))["hand"], json!("right"));
}

/// A file from a newer program, one that is not a settings file, a part the file lacks or a
/// key naming no command refuses the import, and changes nothing.
#[test]
fn an_import_is_refused_whole_and_changes_nothing() {
    let (mut ui, _, file) = window("export-refused");
    let path = file.to_string_lossy().to_string();
    let import = |ui: &mut Ui, text: &str, parts: Value| {
        std::fs::write(&file, text).unwrap_or_else(|e| panic!("{e}"));
        ui.err("ui.layout", json!({ "import": path, "parts": parts }))
    };
    let all = json!(["arrangement", "quick", "keymap"]);
    let (code, message) = import(&mut ui, "version = 2\n", all.clone());
    assert_eq!(code, ErrorCode::Precondition, "{message}");
    assert!(
        message.contains("newer GigaSculpt (version 2)"),
        "{message}"
    );
    let (code, message) = import(&mut ui, "[quick]\nshown = 1\n", all);
    assert_eq!(code, ErrorCode::Precondition, "no version: {message}");
    let (code, message) = import(&mut ui, "version = 1\n[quick\n", json!([]));
    assert_eq!(code, ErrorCode::Damaged, "{message}");
    assert!(message.contains("line 2"), "names the line: {message}");
    let quick = "version = 1\n[quick]\nshown = 1\nmenus = [[\"history\"]]\n";
    let (code, message) = import(&mut ui, quick, json!(["keymap"]));
    assert_eq!(code, ErrorCode::NotFound, "{message}");
    assert!(message.contains("quick"), "names what it holds: {message}");
    let unknown =
        format!("{quick}[[keymap.bind]]\nkey = \"Ctrl+K\"\ncommand = \"primitive.donut\"\n");
    let before = menus(&mut ui);
    let (code, message) = import(&mut ui, &unknown, json!(["quick", "keymap"]));
    assert_eq!(code, ErrorCode::NotFound, "{message}");
    assert!(message.contains("primitive.donut"), "{message}");
    assert_eq!(
        menus(&mut ui),
        before,
        "the quick part waits for the keymap's"
    );
    assert_eq!(runs(&ui, "Ctrl+K"), None);

    let parts = json!({ "parts": ["keymap"] });
    assert_eq!(ui.err("ui.layout", parts).0, ErrorCode::ArgsInvalid);
    let both = json!({ "export": path, "name": "workbench" });
    assert_eq!(ui.err("ui.layout", both).0, ErrorCode::ArgsInvalid);
    let gone = json!({ "import": file.with_file_name("none.toml").to_string_lossy() });
    assert_eq!(ui.err("ui.layout", gone).0, ErrorCode::NotFound);
}
