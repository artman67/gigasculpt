//! The held keys around a stroke (`brush-settings.modifier-key-slots`, gs-ui src/held.rs): a
//! stroke keeps the slot it pressed with until the pen lifts, and the cursor says what the
//! stroke does, whatever slot keys go down or up after the press, and names the line while Shift
//! holds the stroke to one (`stroke.freehand`); Alt works the other way from
//! the sample it goes down on, and says so. Ctrl pressed mid-stroke holds the Mask slot from the
//! lift; Ctrl+Z with Ctrl still down from a mask stroke undoes it.

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::rig::{Window, across, log_path, pen_at, with_sphere};

pub fn keys(shift: bool, ctrl: bool, alt: bool) -> Modifiers {
    Modifiers {
        shift,
        ctrl,
        alt,
        ..Modifiers::default()
    }
}

/// The cursor's line for the held keys (`Shift: Smooth`), if it shows one.
fn says(w: &mut Window) -> Vec<String> {
    let keys = ["Shift", "Ctrl", "Alt"];
    (w.cursor_lines().into_iter())
        .filter(|l| keys.iter().any(|k| l.starts_with(k)))
        .collect()
}

/// The pen hovering at `x`, `y` after `ms` milliseconds, then pressing there.
fn press(w: &mut Window, (x, y): (f32, f32), ms: u64) {
    w.pen(pen_at(ms, x, y, 0.0));
    w.pen(pen_at(ms + 4, x, y, 0.5));
}

#[test]
fn the_cursor_says_what_the_stroke_does_while_keys_change_under_it() {
    let (mut w, rect) = with_sphere(None);
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    // A Draw stroke: Shift after the press holds it to a line, not the Smooth slot, so the
    // cursor names the line and no slot; Alt then subtracts, and says so.
    press(&mut w, (cx - 40.0, cy), 0);
    w.pen(pen_at(12, cx - 30.0, cy, 0.5));
    w.input(&[], keys(true, false, false));
    w.pen(pen_at(16, cx - 20.0, cy, 0.5));
    assert_eq!(says(&mut w), ["Shift: Line"], "Shift holds a line");
    w.input(&[], keys(true, false, true));
    w.pen(pen_at(20, cx - 10.0, cy, 0.5));
    let both = ["Alt: Subtract", "Shift: Line"];
    assert_eq!(says(&mut w), both, "Alt works mid-stroke");
    w.pen(pen_at(24, cx - 10.0, cy, 0.0));
    // With the pen up, the keys hold their slot again.
    w.input(&[], keys(true, false, false));
    assert_eq!(says(&mut w), ["Shift: Smooth"], "the pen up");
    w.input(&[], Modifiers::default());
    // A Ctrl stroke masks: Shift after the press holds its line and calls no Select slot.
    w.input(&[], keys(false, true, false));
    press(&mut w, (cx - 40.0, cy + 30.0), 40);
    w.pen(pen_at(52, cx - 30.0, cy + 30.0, 0.5));
    w.input(&[], keys(true, true, false));
    w.pen(pen_at(56, cx - 20.0, cy + 30.0, 0.5));
    assert_eq!(
        says(&mut w),
        ["Ctrl: Mask Pen", "Shift: Line"],
        "Shift holds the Mask Pen's line"
    );
    w.pen(pen_at(60, cx - 20.0, cy + 30.0, 0.0));
    w.input(&[], keys(true, true, false));
    assert_eq!(says(&mut w), ["Ctrl+Shift: Select Rect"], "the pen up");
}

/// The slot the engine holds now.
fn held_slot(w: &mut Window) -> Value {
    w.ok("session.state", json!({}))["brush.slot.held"].clone()
}

/// Late keys belong to the stroke (`interface.gesture-map-model`): Ctrl pressed mid-stroke
/// leaves a Draw stroke drawing, and holds the Mask slot once the pen lifts.
#[test]
fn ctrl_pressed_mid_stroke_holds_the_mask_slot_from_the_lift() {
    let (mut w, rect) = with_sphere(Some(log_path("ctrl-late")));
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    let ctrl = keys(false, true, false);
    press(&mut w, (cx - 40.0, cy), 0);
    w.pen(pen_at(12, cx - 30.0, cy, 0.5));
    w.input(&[], ctrl);
    w.pen(pen_at(16, cx - 20.0, cy, 0.5));
    assert_eq!(held_slot(&mut w), Value::Null, "no slot mid-stroke");
    assert_eq!(says(&mut w), Vec::<String>::new(), "nor on the cursor");
    // The lift, and the frames the window draws after it: the slot comes on the one after.
    w.pen(pen_at(20, cx - 10.0, cy, 0.0));
    w.wait(0.05);
    let logged = w.logged();
    let stroke = logged.iter().find(|l| l.0 == "brush.stroke");
    assert_eq!(stroke.map(|l| l.1["brush"].clone()), Some(json!("Draw")));
    assert_eq!(held_slot(&mut w), "mask", "held from the lift");
    assert_eq!(says(&mut w), ["Ctrl: Mask Pen"]);
}

/// Ctrl+Z with Ctrl still down from a mask stroke undoes the stroke, and Ctrl still holds the
/// Mask slot.
#[test]
fn ctrl_z_right_after_a_ctrl_stroke_undoes_it() {
    let (mut w, rect) = with_sphere(None);
    let before = w.hash();
    let ctrl = keys(false, true, false);
    w.input(&[], ctrl);
    for s in across(rect, 6) {
        w.pen(s);
    }
    assert_ne!(w.hash(), before, "the stroke masks");
    let z = |down| Input::Key {
        key: "Z".into(),
        down,
        repeat: false,
        modifiers: ctrl,
    };
    w.input(&[z(true)], ctrl);
    w.input(&[z(false)], ctrl);
    assert_eq!(w.hash(), before, "Ctrl+Z undoes the mask stroke");
    assert_eq!(held_slot(&mut w), "mask", "Ctrl still holds the Mask slot");
    w.input(&[], Modifiers::default());
    let state = w.ok("session.state", json!({}));
    assert_eq!(
        (state["brush"].clone(), state["brush.slot.held"].clone()),
        (json!("Draw"), Value::Null)
    );
}
