//! The held keys' brush slots (`brush-settings.modifier-key-slots`): Shift, Ctrl and Shift+Ctrl
//! hold the Smooth, Mask and Select slots in the engine (`brush.slot`), a brush chosen or a
//! slider changed while one is held works on the slot's brush, the slot's brush shows on the
//! cursor and the status line, a Smooth brush picked goes on the Smooth slot, and the Mask or
//! Select keys tapped again step to the next brush of their family.

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, held};

fn both() -> Modifiers {
    Modifiers {
        shift: true,
        ctrl: true,
        ..Modifiers::default()
    }
}

/// `ui` with the pen over the 3D view, so the cursor shows.
fn hovering() -> Ui {
    let mut ui = Ui::new();
    let [x, y] = ui.middle();
    ui.pen(x, y);
    ui
}

/// The cursor and the status line both say `text`, with the pen brought back over the 3D view
/// (a press on a panel leaves it there).
fn says(ui: &mut Ui, text: &str) {
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let (cursor, status) = (ui.labels("viewport"), ui.labels("status"));
    assert!(cursor.iter().any(|l| l == text), "cursor: {cursor:?}");
    assert!(status.iter().any(|l| l == text), "status line: {status:?}");
}

/// The catalog's check: Flat Polish chosen while Shift is held goes on the Smooth slot and
/// Smooth is gone until reassigned, and Strength changed while it is held leaves Draw's.
#[test]
fn a_brush_chosen_or_a_slider_changed_while_a_slot_is_held_works_on_the_slot() {
    let mut ui = hovering();
    let draw = ui.state()["brush.strength"].clone();
    ui.input(&[], held("Shift"));
    let hold = ui.logged("brush.slot");
    assert_eq!(hold.len(), 1, "one call as Shift goes down: {hold:?}");
    assert_eq!(hold[0]["role"], "human");
    assert_eq!(ui.state()["brush.slot.held"], "smooth");
    says(&mut ui, "Shift: Smooth");
    ui.pick("Flat Polish");
    says(&mut ui, "Shift: Flat Polish");
    // The Strength slider dragged from its rail to its left end.
    let r = ui.rect("brush > Strength");
    let [_, y] = middle(r);
    let on_rail = r[0] + 20.0;
    for (x, down) in [
        (on_rail, false),
        (on_rail, true),
        (r[0] - 40.0, true),
        (r[0] - 40.0, false),
    ] {
        ui.touch(x, y, down);
    }
    let slot_strength = ui.state()["brush.strength"].clone();
    let low = slot_strength.as_f64().is_some_and(|s| s < 0.01);
    assert!(low, "Flat Polish's own, at the rail's end: {slot_strength}");
    ui.input(&[], Modifiers::default());
    let st = ui.state();
    assert_eq!(st["brush"], "Draw", "the main brush is back");
    assert_eq!(st["brush.strength"], draw, "Draw's Strength is untouched");
    assert_eq!(st["brush.slot.smooth"], "Flat Polish");
    assert_eq!(st["brush.slot.held"], Value::Null);
    ui.input(&[], held("Shift"));
    says(&mut ui, "Shift: Flat Polish");
    assert_eq!(
        ui.state()["brush.strength"],
        slot_strength,
        "the slot's own"
    );
    ui.input(&[], held("Ctrl"));
    ui.pick("Inflate");
    ui.input(&[], Modifiers::default());
    let st = ui.state();
    assert_eq!(st["brush"], "Draw");
    assert_eq!(st["brush.slot.mask"], "Inflate", "any brush on any slot");
}

/// A Smooth brush picked with no key held goes on the Smooth slot and lights there; the main
/// brush stays.
#[test]
fn a_smooth_brush_picked_goes_on_the_smooth_slot() {
    let mut ui = hovering();
    ui.pick("Smooth Polish");
    let st = ui.state();
    assert_eq!(st["brush"], "Draw");
    assert_eq!(st["brush.slot.smooth"], "Smooth Polish");
    ui.open_family("Smooth");
    let family = ui.widgets("family");
    let lit = |label: &str| {
        let w = family
            .iter()
            .find(|w| w["label"] == label && w["role"] == "toggle");
        w.map(|w| w["selected"].clone()).unwrap_or_default()
    };
    assert_eq!(
        (lit("Smooth Polish"), lit("Smooth")),
        (json!(true), json!(false))
    );
    ui.input(&[], held("Shift"));
    says(&mut ui, "Shift: Smooth Polish");
}

/// Shift+Ctrl holds the Select slot (`interface.gesture-map-model`); its keys let go and
/// pressed again at once step it through its family, and pressed again later do not. Shift's
/// Smooth slot never steps.
#[test]
fn shift_ctrl_holds_the_select_slot_and_a_tap_steps_it() {
    let mut ui = hovering();
    ui.input(&[], both());
    assert_eq!(ui.state()["brush.slot.held"], "select");
    says(&mut ui, "Ctrl+Shift: Select Rect");
    ui.input(&[], Modifiers::default());
    ui.input(&[], both());
    says(&mut ui, "Ctrl+Shift: Clip Curve");
    ui.input(&[], Modifiers::default());
    ui.wait(0.5);
    ui.input(&[], both());
    says(&mut ui, "Ctrl+Shift: Clip Curve");
    ui.input(&[], Modifiers::default());
    ui.input(&[], held("Shift"));
    ui.input(&[], Modifiers::default());
    ui.input(&[], held("Shift"));
    says(&mut ui, "Shift: Smooth");
}
