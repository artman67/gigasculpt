//! What shows (interface.layouts): with one section open at a time, a plain tap on a title that
//! opens it shuts the others in its tray and a Shift tap keeps them open; each tray hides and
//! shows on its own, mirrored with the hand, and `layout.toml` keeps it; minimal chrome hides
//! the shelf, the trays and the header, and showing them again puts every panel back where
//! it was.

use gs_platform::Modifiers;
use gs_ui::LayoutFile;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, folder, held};

/// Whether the scene tray's section `id` is open, from the layout: two open sections can be
/// taller than the tray, which then shows only part of them.
fn open(ui: &mut Ui, id: &str) -> bool {
    ui.settle();
    !ui.host.lock().shell.layout.layout.is_folded(id)
}

/// A tap on the title row of the scene tray's section `id`, holding `held`.
fn tap_title(ui: &mut Ui, id: &str, held: Modifiers) {
    let rect = ui.rect(&format!("tray > {id}"));
    ui.input(&[], held);
    ui.tap_at(middle(rect)[0], rect[1] + 10.0);
    ui.input(&[], Modifiers::default());
}

#[test]
fn a_shift_tap_on_a_second_title_keeps_the_first_open_and_a_plain_tap_shuts_it() {
    let mut ui = Ui::new();
    assert_eq!(ui.state()["layout.one_open"], json!(true), "on by default");
    ui.open_only(&["detail"]);
    tap_title(&mut ui, "history", held("Shift"));
    assert!(
        open(&mut ui, "detail") && open(&mut ui, "history"),
        "Shift keeps both"
    );
    // Shut again, then opened with a plain tap: the other open section shuts.
    tap_title(&mut ui, "history", Modifiers::default());
    assert!(open(&mut ui, "detail") && !open(&mut ui, "history"));
    tap_title(&mut ui, "history", Modifiers::default());
    assert!(
        !open(&mut ui, "detail") && open(&mut ui, "history"),
        "one open at a time"
    );
    // Shutting one never opens another, and the brush tray keeps its own.
    assert!(!ui.host.lock().shell.layout.layout.is_folded("brush"));

    // With the setting off, a plain tap keeps the others open too.
    ui.ok("ui.layout", json!({ "one_open": false }));
    assert_eq!(ui.state()["layout.one_open"], json!(false));
    tap_title(&mut ui, "detail", Modifiers::default());
    assert!(open(&mut ui, "detail") && open(&mut ui, "history"));
}

/// The panels `ui.describe` lists, each by id with its rectangle.
fn panels(ui: &mut Ui) -> Vec<(String, Value)> {
    let d = ui.describe();
    let list = d["panels"].as_array().cloned().unwrap_or_default();
    let id = |p: &Value| p["id"].as_str().unwrap_or_default().to_string();
    list.iter().map(|p| (id(p), p["rect"].clone())).collect()
}

/// The ids among `panels`.
fn ids(panels: &[(String, Value)]) -> Vec<&str> {
    panels.iter().map(|(id, _)| id.as_str()).collect()
}

#[test]
fn hiding_the_left_tray_leaves_the_right_and_layout_toml_keeps_it() {
    let path = folder("hidden").join("layout.toml");
    let mut ui = Ui::with_layout(LayoutFile::load(&path).unwrap_or_else(|e| panic!("{e}")));
    let scene = ui.rect("tray");
    let shown = |ui: &mut Ui| {
        let s = ui.state();
        (s["layout.left"].clone(), s["layout.right"].clone())
    };
    assert_eq!(shown(&mut ui), (json!(true), json!(true)));
    ui.ok("ui.layout", json!({ "left": false }));
    let now = panels(&mut ui);
    assert!(!ids(&now).contains(&"brush"), "{:?}", ids(&now));
    assert_eq!(ui.rect("tray"), scene, "the right tray is unchanged");
    assert_eq!(shown(&mut ui), (json!(false), json!(true)));
    let saved = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(saved.contains("hidden = [\"brush\"]"), "{saved}");

    // Mirrored, the hidden tray goes with its side: the brush tray is on the right now.
    ui.ok("ui.layout", json!({ "hand": "left" }));
    assert_eq!(shown(&mut ui), (json!(true), json!(false)));
    ui.ok("ui.layout", json!({ "hand": "right" }));

    // Header > Window's Left tray shows it again, lit while it shows.
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Left tray");
    assert_eq!(shown(&mut ui), (json!(true), json!(true)));
    assert!(ids(&panels(&mut ui)).contains(&"brush"));
}

#[test]
fn the_minimal_chrome_toggle_twice_restores_every_panel_exactly() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "right": false }));
    let before = panels(&mut ui);
    for id in ["shelf", "brush", "header"] {
        assert!(ids(&before).contains(&id), "{:?}", ids(&before));
    }
    ui.ok("ui.layout", json!({ "chrome": false }));
    let bare = panels(&mut ui);
    assert_eq!(ui.state()["layout.chrome"], json!(false));
    for id in ["shelf", "brush", "tray", "header"] {
        assert!(!ids(&bare).contains(&id), "{id} hides: {:?}", ids(&bare));
    }
    assert!(
        ids(&bare).contains(&"status"),
        "the status line stays, saying so"
    );
    let status = ui.labels("status").join(" ");
    assert!(status.contains("Panels hidden"), "{status}");
    ui.ok("ui.layout", json!({ "chrome": true }));
    assert_eq!(panels(&mut ui), before, "every panel back where it was");
    assert_eq!(
        ui.state()["layout.right"],
        json!(false),
        "the hidden tray stays hidden"
    );
}
