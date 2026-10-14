//! Arrangements (decision 0025, W3): a tray's inner edge drags it wider or narrower, as far as
//! leaves the 3D view half the window, and `layout.toml` keeps the width;
//! Ctrl+Shift+1 to 3 save the arrangement shown as a layout file and Ctrl+1 to 3 show it.

use gs_platform::Modifiers;
use gs_protocol::ErrorCode;
use gs_ui::LayoutFile;
use serde_json::json;

use crate::rig::{SIZE, Ui, folder, held};

/// A window whose layout state is the file `path`.
fn window(path: &std::path::Path) -> Ui {
    Ui::with_layout(LayoutFile::load(path).unwrap_or_else(|e| panic!("{e}")))
}

/// The panel `id`'s width as `ui.describe` reports it (inside its margins).
fn width(ui: &mut Ui, id: &str) -> f64 {
    let rect = ui.rect(id);
    rect[2] - rect[0]
}

/// Drags the inner edge of the tray `id` with the pen `by` points to the right (less than
/// nothing: to the left).
fn drag_edge(ui: &mut Ui, id: &str, by: f64) {
    let rect = ui.rect(id);
    let margin = f64::from(ui.host.lock().theme().spacing.margin);
    // The edge facing the 3D view, just outside the tray's margin.
    let x = if rect[0] < f64::from(SIZE[0]) / 2.0 {
        rect[2] + margin
    } else {
        rect[0] - margin
    };
    let y = f64::midpoint(rect[1], rect[3]);
    for (at, down) in [(x, false), (x, true), (x + by, true), (x + by, false)] {
        ui.touch(at, y, down);
    }
}

#[test]
fn a_trays_inner_edge_drags_it_wider_or_narrower_and_layout_toml_keeps_the_width() {
    let path = folder("widths").join("layout.toml");
    let mut ui = window(&path);
    let (scene, brush) = (width(&mut ui, "tray"), width(&mut ui, "brush"));
    drag_edge(&mut ui, "tray", -60.0);
    drag_edge(&mut ui, "brush", -40.0);
    let wider = width(&mut ui, "tray");
    let narrower = width(&mut ui, "brush");
    assert!(
        (wider - scene - 60.0).abs() <= 1.0,
        "scene tray {scene} -> {wider}"
    );
    assert!(
        (brush - narrower - 40.0).abs() <= 1.0,
        "brush tray {brush} -> {narrower}"
    );

    // Dragged far into the 3D view, the tray stops where the view keeps half the window.
    drag_edge(&mut ui, "tray", -1000.0);
    let (tray, left) = (ui.rect("tray"), ui.rect("brush"));
    assert!(tray[2] - tray[0] > wider, "it widened");
    assert!(
        tray[0] - left[2] >= f64::from(SIZE[0]) / 2.0,
        "the view keeps half: {left:?} {tray:?}"
    );
    let widest = width(&mut ui, "tray");

    // A new window with the same layout file shows the trays where they were left.
    drop(ui);
    let mut again = window(&path);
    let kept = (width(&mut again, "tray"), width(&mut again, "brush"));
    assert_eq!(kept, (widest, narrower), "layout.toml kept the widths");
}

#[test]
fn ctrl_shift_1_saves_the_arrangement_shown_and_ctrl_1_shows_it_again() {
    let dir = folder("saved");
    let mut ui = window(&dir.join("layout.toml"));
    ui.host.lock().shell.layouts = Some(dir.join("layouts"));
    let workbench = width(&mut ui, "tray");
    drag_edge(&mut ui, "tray", -80.0);
    let wider = width(&mut ui, "tray");

    let ctrl_shift = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::default()
    };
    ui.tap("1", ctrl_shift);
    let problem = ui.host.lock().shell.status.problem.clone();
    let file = dir.join("layouts").join("arrangement-1.toml");
    assert!(file.exists(), "saved: {problem:?}");
    let shown = ui.ok("ui.layout", json!({}));
    assert_eq!(
        (&shown["name"], &shown["title"]),
        (&json!("arrangement-1"), &json!("Arrangement 1"))
    );
    assert_eq!(width(&mut ui, "tray"), wider, "shown as it was arranged");

    // The Workbench shows at its own widths; Ctrl+1 brings the arrangement back.
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    assert_eq!(width(&mut ui, "tray"), workbench);
    ui.tap("1", held("Ctrl"));
    assert_eq!(
        width(&mut ui, "tray"),
        wider,
        "Ctrl+1 shows the saved width"
    );

    // A shipped file's name is refused, so an arrangement never hides the Workbench.
    let (_, message) = ui.err("ui.layout", json!({ "save": "workbench" }));
    assert!(message.contains("shipped"), "{message}");
}

#[test]
fn save_refuses_a_name_that_is_not_a_plain_file_name_or_comes_with_name() {
    let mut ui = Ui::new();
    for save in ["../keymap", "Arrangement", ""] {
        let (code, _) = ui.err("ui.layout", json!({ "save": save }));
        assert_eq!(code, ErrorCode::ArgsInvalid, "{save:?}");
    }
    let both = json!({ "name": "workbench", "save": "arrangement-1" });
    assert_eq!(ui.err("ui.layout", both).0, ErrorCode::ArgsInvalid);
}
