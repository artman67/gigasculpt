//! Layout presets (interface.layout-presets): Next and Previous step through the layouts in
//! their order and wrap, keeping the theme shown; Restore shows the default layout as shipped,
//! keeping the hand, the scale and the theme.

use gs_engine::Hand;
use gs_protocol::ErrorCode;
use gs_ui::{Layout, LayoutFile};
use serde_json::json;

use crate::rig::{Ui, folder};

/// A window whose saved arrangements go to a fresh folder `name`.
fn saving(name: &str) -> Ui {
    let dir = folder(name);
    let ui = Ui::with_layout(
        LayoutFile::load(&dir.join("layout.toml")).unwrap_or_else(|e| panic!("{e}")),
    );
    ui.host.lock().shell.layouts = Some(dir.join("layouts"));
    ui
}

#[test]
fn next_steps_through_three_layouts_back_to_the_first_and_keeps_the_theme() {
    let mut ui = saving("step");
    // Two arrangements saved under the dark theme, then the light one shown.
    ui.ok("ui.layout", json!({ "save": "arrangement-1" }));
    ui.ok("ui.layout", json!({ "save": "arrangement-2" }));
    ui.ok(
        "ui.layout",
        json!({ "name": "workbench", "theme": "light" }),
    );
    assert_eq!(
        ui.layouts(),
        ["workbench", "arrangement-1", "arrangement-2"]
    );
    let step = |ui: &mut Ui, by: i32| ui.ok("ui.layout", json!({ "step": by }));
    assert_eq!(step(&mut ui, 1)["name"], json!("arrangement-1"));
    assert_eq!(step(&mut ui, 1)["name"], json!("arrangement-2"));
    let back = step(&mut ui, 1);
    assert_eq!(back["name"], json!("workbench"), "wraps to the first");
    assert_eq!(
        back["theme"],
        json!("light"),
        "a layout keeps the colors shown"
    );
    assert_eq!(
        step(&mut ui, -1)["name"],
        json!("arrangement-2"),
        "and back"
    );

    // Header > Window's Next layout does the same.
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Next");
    assert_eq!(ui.state()["layout"], json!("workbench"));

    for args in [
        json!({ "step": 2 }),
        json!({ "step": 1, "name": "workbench" }),
    ] {
        assert_eq!(
            ui.err("ui.layout", args.clone()).0,
            ErrorCode::ArgsInvalid,
            "{args}"
        );
    }
}

#[test]
fn restore_shows_the_default_layout_as_shipped_and_keeps_the_hand_and_theme() {
    let mut ui = saving("restore");
    ui.ok("ui.layout", json!({ "save": "arrangement-1" }));
    let args = json!({ "hand": "left", "theme": "light", "right": false, "one_open": false });
    ui.ok("ui.layout", args);
    ui.host
        .lock()
        .shell
        .layout
        .layout
        .widths
        .insert("tray".into(), 420.0);
    // The scene tray is on the left now, and still shows; Header > Window holds Restore.
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Restore");
    let restored = ui.host.lock().shell.layout.layout.clone();
    let shipped = Layout::default();
    assert_eq!(ui.state()["layout"], json!("workbench"));
    assert_eq!(restored.folded, shipped.folded);
    assert!(restored.widths.is_empty() && restored.hidden.is_empty());
    assert!(restored.one_open);
    assert_eq!(
        (restored.hand, restored.theme.as_str()),
        (Hand::Left, "light")
    );

    // Minimal chrome hides the button; the command restores the panels too.
    ui.ok("ui.layout", json!({ "chrome": false }));
    let info = ui.ok("ui.layout", json!({ "restore": true }));
    assert_eq!(info["chrome"], json!(true));
    let both = json!({ "restore": true, "name": "arrangement-1" });
    assert_eq!(ui.err("ui.layout", both).0, ErrorCode::ArgsInvalid);
}
