//! The quick menu (decision 0025, section 2): Shift+Space opens it with its corner at the
//! pointer, its controls run their commands and leave it open, and Escape or a press outside
//! closes it; a slider in it scrubs in place; what it holds lasts after a restart; with two
//! menus, holding Shift+Space and swiping across shows both in order.

use gs_platform::Modifiers;
use gs_protocol::ErrorCode;
use gs_ui::LayoutFile;
use serde_json::json;

use crate::region::middle;
use crate::rig::{Ui, folder, held, key};

/// Where the quick menu shows, in pixels, if it is open.
fn menu(ui: &mut Ui) -> Option<[f64; 4]> {
    let d = ui.describe();
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let panel = panels.into_iter().find(|p| p["id"] == "quick")?;
    Some(std::array::from_fn(|i| {
        panel["rect"][i].as_f64().unwrap_or_default()
    }))
}

#[test]
fn the_quick_menu_opens_at_the_pointer_and_runs_its_controls() {
    let mut ui = Ui::new();
    ui.settle();
    assert_eq!(menu(&mut ui), None, "it starts shut");
    let [x0, y0, x1, y1] = ui.rect("viewport");
    let at = [x0 + (x1 - x0) * 0.3, y0 + (y1 - y0) * 0.3];
    ui.pen(at[0], at[1]);
    ui.tap("Space", held("Shift"));
    let rect = menu(&mut ui).expect("Shift+Space opens it");
    assert!(
        !ui.host.lock().shell.palette.open,
        "not the palette, on Space"
    );
    let corner = [rect[0], rect[1]];
    assert!(
        (corner[0] - at[0]).abs() < 2.0 && (corner[1] - at[1]).abs() < 2.0,
        "its corner {corner:?} is at the pointer {at:?}"
    );
    // Symmetry's switches read as their set draws them, `Symmetry [On] [X][Y][Z]`.
    for label in ["Size", "Symmetry", "On", "X", "Invert mask", "Undo"] {
        assert!(
            ui.labels("quick").iter().any(|l| l == label),
            "it holds {label}"
        );
    }
    // The brush's three sliders by the names the tray gives them, and nothing else of Basics
    // (decision 0060, section 10): the falloff is Softness everywhere (amended for UI-finish),
    // and a slider's own label is its only title (amended for UI-last).
    let sliders: Vec<String> = (ui.widgets("quick").into_iter())
        .filter(|w| w["role"] == "slider")
        .filter_map(|w| Some(w["label"].as_str()?.to_string()))
        .collect();
    assert_eq!(sliders, ["Size", "Strength", "Softness"]);
    let labels = ui.labels("quick");
    for basics in ["Basics", "Add", "Subtract", "Pressure…"] {
        assert!(!labels.iter().any(|l| l == basics), "{basics}: {labels:?}");
    }
    let softness = labels.iter().filter(|l| *l == "Softness").count();
    assert_eq!(softness, 1, "{labels:?}");
    let mirror = |ui: &mut Ui| ui.state()["symmetry.x"].clone();
    assert_eq!(mirror(&mut ui), json!(false));
    let [x, y] = middle(ui.rect("quick > X"));
    ui.tap_at(x, y);
    assert_eq!(mirror(&mut ui), json!(true), "X ran from the menu");
    assert!(menu(&mut ui).is_some(), "it stays open while it is used");
    ui.tap("Escape", Modifiers::default());
    assert_eq!(menu(&mut ui), None, "Escape closes it");
    // Opened by its command, as an agent or a barrel click does, and closed by a press outside.
    let elsewhere = [x0 + (x1 - x0) * 0.6, y0 + (y1 - y0) * 0.6];
    ui.pen(at[0], at[1]);
    assert_eq!(ui.ok("ui.quick", json!({ "open": true }))["open"], true);
    ui.settle();
    assert!(menu(&mut ui).is_some(), "ui.quick opens it");
    ui.tap_at(elsewhere[0], elsewhere[1]);
    assert_eq!(menu(&mut ui), None, "a press outside closes it");
}

/// Decision 0060 as amended for UI-tidy: the quick menu stays inside the 3D view. In a 1400 by
/// 860 window, opened near the view's foot, it moves up and ends above the status line.
#[test]
fn the_quick_menu_opened_near_the_views_foot_ends_above_the_status_line() {
    let mut ui = Ui::new();
    ui.host.lock().resize([1400, 860], 1.0);
    ui.settle();
    let view = ui.rect("viewport");
    ui.pen(view[0] + (view[2] - view[0]) * 0.3, view[3] - 4.0);
    ui.tap("Space", held("Shift"));
    let rect = menu(&mut ui).expect("Shift+Space opens it");
    let status = ui.rect("status");
    assert!(rect[3] <= status[1], "{rect:?} ends above {status:?}");
    let inside = rect[1] >= view[1] && rect[3] <= view[3];
    assert!(inside, "{rect:?} inside the view {view:?}");
}

/// Decision 0060, section 8, rule 6 (no word repeated): the Symmetry row names itself, so no
/// "Switches" title stands over it, and no title reads twice in the menu.
#[test]
fn the_quick_menus_titles_read_once() {
    let mut ui = Ui::new();
    pointer(&mut ui);
    ui.ok("ui.quick", json!({ "open": true }));
    ui.settle();
    let widgets = ui.widgets("quick");
    let words = |w: &serde_json::Value| w["label"].as_str().map(String::from);
    let all: Vec<String> = widgets.iter().filter_map(words).collect();
    let titles = widgets.iter().filter(|w| w["role"] == "label");
    for title in titles.filter_map(words) {
        assert_ne!(title, "Switches", "{all:?}");
        let times = all.iter().filter(|l| **l == title).count();
        assert_eq!(times, 1, "{title} reads once: {all:?}");
    }
}

/// The pen at a third of the way across and down the 3D view, where the menu opens.
pub(crate) fn pointer(ui: &mut Ui) -> [f64; 2] {
    let [x0, y0, x1, y1] = ui.rect("viewport");
    let at = [x0 + (x1 - x0) * 0.3, y0 + (y1 - y0) * 0.3];
    ui.pen(at[0], at[1]);
    at
}

#[test]
fn a_slider_in_the_quick_menu_scrubs_the_brush_size_in_place() {
    let mut ui = Ui::new();
    pointer(&mut ui);
    ui.ok("ui.quick", json!({ "open": true }));
    ui.settle();
    let size = |ui: &mut Ui| ui.state()["brush.size_px"].as_f64().unwrap_or_default();
    let before = size(&mut ui);
    let [x0, _, x1, _] = ui.rect("quick > Size");
    let [mid, y] = middle(ui.rect("quick > Size"));
    ui.touch(mid, y, false);
    ui.touch(mid, y, true);
    ui.touch(x0 + (x1 - x0) * 0.9, y, true);
    ui.touch(x0 + (x1 - x0) * 0.9, y, false);
    assert!(size(&mut ui) > before, "dragged right: {before} grew");
    assert!(menu(&mut ui).is_some(), "the menu stays open");
}

#[test]
fn an_added_item_stays_in_the_quick_menu_after_a_restart() {
    let path = folder("quick-restart").join("layout.toml");
    let load = || LayoutFile::load(&path).unwrap_or_else(|e| panic!("{e}"));
    let mut ui = Ui::with_layout(load());
    let added = ui.ok("ui.quick", json!({ "add": "view" }));
    assert_eq!(
        added["menus"][0].as_array().and_then(|m| m.last()),
        Some(&json!("view"))
    );
    let unknown = ui.err("ui.quick", json!({ "add": "nothing-here" }));
    assert_eq!(unknown.0, ErrorCode::NotFound, "{}", unknown.1);
    ui.settle(); // the frame after a change saves the layout state
    drop(ui);

    // The window again, from the file.
    let mut ui = Ui::with_layout(load());
    pointer(&mut ui);
    ui.ok("ui.quick", json!({ "open": true }));
    ui.settle();
    assert!(
        ui.labels("quick").iter().any(|l| l == "Frame"),
        "the view set is still in it"
    );
    // Reset puts back the shipped contents.
    let reset = ui.ok("ui.quick", json!({ "reset": true }));
    assert!(
        !reset["menus"][0]
            .as_array()
            .is_some_and(|m| m.contains(&json!("view")))
    );
}

#[test]
fn holding_the_open_key_and_swiping_shows_two_menus_in_order() {
    let mut ui = Ui::new();
    let built = ui.ok("ui.quick", json!({ "menu": 2, "add": "history" }));
    assert_eq!(built["menus"].as_array().map(Vec::len), Some(2));
    ui.ok("ui.quick", json!({ "menu": 1 }));
    let at = pointer(&mut ui);
    let shift = held("Shift");
    ui.input(&[key("Space", true, shift)], shift);
    let shown = |ui: &mut Ui| ui.ok("ui.quick", json!({}))["menu"].clone();
    assert!(menu(&mut ui).is_some(), "Shift+Space opens it");
    assert_eq!(shown(&mut ui), json!(1));
    assert!(
        ui.labels("quick").iter().any(|l| l == "Symmetry"),
        "menu 1 first"
    );
    // Held, a swipe to the right shows the next menu, and one back to the left the first.
    ui.pen(at[0] + 100.0, at[1]);
    ui.settle();
    assert_eq!(shown(&mut ui), json!(2), "then menu 2");
    let labels = ui.labels("quick");
    assert!(labels.iter().any(|l| l == "Undo") && !labels.iter().any(|l| l == "Symmetry"));
    ui.pen(at[0], at[1]);
    ui.settle();
    assert_eq!(shown(&mut ui), json!(1), "and back");
    // Let go, a swipe does nothing; the menu's buttons show either menu.
    ui.input(&[key("Space", false, shift)], Modifiers::default());
    ui.pen(at[0] + 100.0, at[1]);
    assert_eq!(shown(&mut ui), json!(1));
    ui.press("quick > Menu 2");
    assert_eq!(shown(&mut ui), json!(2));
    let past = ui.err("ui.quick", json!({ "menu": 4 }));
    assert_eq!(past.0, ErrorCode::NotFound, "{}", past.1);
}
