//! The wheel while the quick menu shows (catalog `interface.quick-menu`, decision 0025 section 2
//! as amended 2026-10-12): an unbound notch steps the brush's Size wherever the pointer is, a
//! part notch waiting for the rest, and never dollies; a notch the keymap binds runs its
//! binding; over one of the menu's sliders the notch moves that slider; with the menu shut the
//! wheel dollies again.

use gs_platform::{Input, Modifiers};
use serde_json::json;

use crate::region::middle;
use crate::rig::{Ui, held};

/// The brush's size, in pixels.
fn size(ui: &mut Ui) -> f64 {
    ui.state()["brush.size_px"].as_f64().unwrap_or_default()
}

/// The wheel turned `dy` notches with `held`.
fn notch(ui: &mut Ui, dy: f32, held: Modifiers) {
    ui.input(&[Input::Wheel { dx: 0.0, dy }], held);
}

/// Catalog `interface.quick-menu`: "scrolling the wheel right after opening changes the brush
/// size".
#[test]
fn the_wheel_right_after_opening_the_quick_menu_changes_the_brush_size() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let start = size(&mut ui);
    ui.tap("Space", held("Shift"));
    assert!(!ui.widgets("quick").is_empty(), "Shift+Space opens it");
    notch(&mut ui, 1.0, Modifiers::default());
    let grown = size(&mut ui);
    assert_eq!(grown, start + 8.0, "a notch up is one size step");
    notch(&mut ui, -0.5, Modifiers::default());
    assert_eq!(size(&mut ui), grown, "half a notch waits for the rest");
    notch(&mut ui, -0.5, Modifiers::default());
    assert_eq!(size(&mut ui), start, "and the rest steps it back");
    assert!(ui.logged("view.dolly").is_empty(), "the view never dollies");
    assert!(!ui.widgets("quick").is_empty(), "the menu stays open");

    // With the menu shut the wheel dollies the view, as always.
    ui.tap("Escape", Modifiers::default());
    assert!(ui.widgets("quick").is_empty(), "Escape closes it");
    notch(&mut ui, 1.0, Modifiers::default());
    assert_eq!(size(&mut ui), start);
    assert_eq!(ui.logged("view.dolly").len(), 1, "shut, the wheel dollies");
}

/// A notch the keymap binds runs its binding while the menu shows, and the wheel over one of
/// the menu's sliders moves that slider, not the size.
#[test]
fn a_bound_notch_runs_its_binding_and_a_notch_over_a_slider_moves_it() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let flip =
        json!({ "bind": "Alt+WheelUp", "command": "brush.set", "args": { "flip_mirror": "x" } });
    ui.ok("ui.keymap", flip);
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let start = size(&mut ui);
    ui.tap("Space", held("Shift"));
    notch(&mut ui, 1.0, held("Alt"));
    assert_eq!(ui.state()["symmetry.x"], json!(true), "the bound notch ran");
    assert_eq!(size(&mut ui), start, "and did not step the size");
    ui.input(&[], Modifiers::default());

    let [x, y] = middle(ui.rect("quick > Strength"));
    ui.pen(x, y);
    let strength = |ui: &mut Ui| ui.state()["brush.strength"].as_f64().unwrap_or_default();
    let before = strength(&mut ui);
    notch(&mut ui, 1.0, Modifiers::default());
    let after = strength(&mut ui);
    assert!((after - before - 0.01).abs() < 1e-6, "{before} to {after}");
    assert_eq!(size(&mut ui), start, "the slider under the pointer took it");
}
