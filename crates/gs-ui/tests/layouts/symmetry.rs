//! The symmetry controls (decision 0044) in the 3D view's header (decision 0060, section 6's
//! end): `Symmetry [On] [X][Y][Z] [▼]` at the pen end after Look, the ▼ opening a popover with a
//! `[Mirror|Parallel|Radial]` row per lit axis and Count only while an axis is Radial, and none of
//! it in the brush tray; the X key still flips X. With the pen alone, a barrel click set to the
//! quick menu opens it with the switches only, an axis turns on there, its kind is set in the
//! header's popover, the status line names it, and On in the quick menu switches symmetry off
//! with the axes kept.

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::header::{left_to_right, panel, titles_and_ends};
use crate::library::lit;
use crate::rig::Ui;

/// The status line's symmetry, as `viewport/cursor/lines.rs`' `symmetry_text` writes it.
fn status(ui: &mut Ui) -> String {
    let labels = ui.labels("status");
    let words = ["Mirror", "Parallel", "Radial", "No symmetry"];
    let found = labels
        .into_iter()
        .find(|l| words.iter().any(|w| l.starts_with(w)));
    found.unwrap_or_else(|| panic!("the status line names the symmetry"))
}

/// A barrel click on the 3D view, set to open the quick menu.
fn barrel_click(ui: &mut Ui) {
    let [x0, y0, x1, y1] = ui.rect("viewport");
    let (x, y) = (x0 + (x1 - x0) * 0.3, y0 + (y1 - y0) * 0.3);
    for buttons in [1, 0] {
        ui.ok(
            "pen.input",
            json!({ "samples": [{ "x": x, "y": y, "buttons": buttons }] }),
        );
        ui.settle();
    }
}

/// The symmetry popover's labels, if it shows.
fn popover(ui: &mut Ui) -> Option<Vec<String>> {
    panel(ui, "symmetry-kinds")?;
    Some(ui.labels("symmetry-kinds"))
}

#[test]
fn symmetry_sits_at_the_headers_pen_end_and_its_popover_sets_each_lit_axis_kind() {
    let mut ui = Ui::new();
    let (_, ends) = titles_and_ends(&mut ui);
    let ends = left_to_right(&ends);
    let after = ends
        .iter()
        .position(|l| l == "Look")
        .map(|i| &ends[i + 1..]);
    let want = ["Symmetry", "On", "X", "Y", "Z", "▼"].map(String::from);
    assert_eq!(after, Some(&want[..]), "after Look: {ends:?}");
    let tray = ui.labels("brush");
    for gone in [
        "Symmetry on",
        "Mirror X",
        "X: Mirror",
        "Mirror",
        "Radial count",
    ] {
        assert!(!tray.iter().any(|l| l == gone), "no {gone}: {tray:?}");
    }

    // X lights X, from the header and by its key.
    ui.press("header > X");
    assert_eq!(ui.state()["symmetry.x"], json!(true));
    assert!(lit(&mut ui, "header", "X") && !lit(&mut ui, "header", "Y"));
    ui.tap("X", Modifiers::default());
    assert_eq!(ui.state()["symmetry.x"], json!(false), "the X key flips X");
    ui.tap("X", Modifiers::default());
    assert!(lit(&mut ui, "header", "X"), "and lights it");

    // The ▼ opens a row for the lit axis alone, and no Count while none is Radial.
    assert_eq!(popover(&mut ui), None, "it starts shut");
    ui.press("header > ▼");
    let shown = popover(&mut ui).unwrap_or_else(|| panic!("the ▼ opened the popover"));
    for label in ["X", "Mirror", "Parallel", "Radial"] {
        assert!(shown.iter().any(|l| l == label), "{label}: {shown:?}");
    }
    for label in ["Y", "Z", "Radial count"] {
        assert!(!shown.iter().any(|l| l == label), "no {label}: {shown:?}");
    }
    assert!(lit(&mut ui, "symmetry-kinds", "Mirror"));
    ui.press("symmetry-kinds > Parallel");
    assert_eq!(ui.state()["symmetry.x_type"], json!("parallel"));
    assert!(
        lit(&mut ui, "symmetry-kinds", "Parallel"),
        "it stays open, lit"
    );
    assert!(!lit(&mut ui, "symmetry-kinds", "Mirror"));
    assert_eq!(status(&mut ui), "Parallel X");
    ui.press("symmetry-kinds > Radial");
    let shown = popover(&mut ui).unwrap_or_default();
    let count = shown.iter().any(|l| l == "Radial count");
    assert!(count, "Count, Radial: {shown:?}");

    // A second axis lit in the header gets its row.
    ui.tap("Escape", Modifiers::default());
    assert_eq!(popover(&mut ui), None, "Escape shuts it");
    ui.press("header > Y");
    ui.press("header > ▼");
    let shown = popover(&mut ui).unwrap_or_default();
    assert!(shown.iter().any(|l| l == "Y"), "Y's row: {shown:?}");
    ui.press("header > ▼");
    assert_eq!(popover(&mut ui), None, "a second tap shuts it");
}

#[test]
fn the_pen_alone_sets_an_axis_kind_and_switches_symmetry_off_from_the_quick_menu() {
    let mut ui = Ui::new();
    ui.ok("ui.navigation", json!({ "barrel_click": "quick" }));
    barrel_click(&mut ui);
    assert!(
        lit(&mut ui, "quick", "On"),
        "the barrel click opened the menu, the master on"
    );
    let shown = ui.labels("quick");
    let switches = ["Symmetry", "On", "X", "Y", "Z"];
    assert!(
        switches.iter().all(|s| shown.iter().any(|l| l == s)),
        "{shown:?}"
    );
    let kinds = ["Mirror", "Parallel", "Radial", "Radial count", "▼"];
    assert!(
        !shown.iter().any(|l| kinds.contains(&l.as_str())),
        "the switches only: {shown:?}"
    );
    ui.press("quick > X");

    // The kinds are in the header's popover, a press away.
    ui.press("header > ▼");
    ui.press("symmetry-kinds > Parallel");
    let s = ui.state();
    let x_axis = [&s["symmetry.x"], &s["symmetry.x_type"]];
    assert_eq!(x_axis, [&json!(true), &json!("parallel")], "X on, Parallel");
    assert_eq!(status(&mut ui), "Parallel X");

    // A barrel click opens the menu again.
    ui.tap("Escape", Modifiers::default());
    barrel_click(&mut ui);
    ui.press("quick > On");
    let s = ui.state();
    let kept: [&Value; 3] = [&s["symmetry.on"], &s["symmetry.x"], &s["symmetry.x_type"]];
    assert_eq!(
        kept,
        [&json!(false), &json!(true), &json!("parallel")],
        "off, X kept"
    );
    assert!(!lit(&mut ui, "quick", "On") && lit(&mut ui, "quick", "X"));
    assert!(!lit(&mut ui, "header", "On"), "the header's On is off too");
    assert_eq!(status(&mut ui), "No symmetry", "the pen strokes with none");
}
