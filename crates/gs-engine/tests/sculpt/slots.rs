//! The held keys' brush slots (`brush-settings.modifier-key-slots`): any brush sits in a slot,
//! a brush chosen while the slot is held goes on it and a setting changed then changes only
//! the slot brush, and stepping goes through the slot's family.

use gs_testkit::{Session, code};
use serde_json::{Value, json};

fn slot(s: &mut Session, args: Value) -> Value {
    s.query("brush.slot", args)
}

fn set(s: &mut Session, args: Value) -> Value {
    s.query("brush.set", args)
}

fn state(s: &mut Session) -> Value {
    s.query("session.state", json!({}))
}

/// The catalog's check with the shipped Smooth brushes: Flat Polish chosen while the Smooth
/// slot is held takes the slot from Smooth, and Strength changed then leaves Draw's.
#[test]
fn a_brush_chosen_or_changed_while_a_slot_is_held_works_on_the_slot() {
    let mut s = Session::new();
    let draw = set(&mut s, json!({ "brush": "Draw" }))["strength"].clone();
    let held = slot(&mut s, json!({ "slot": "smooth", "hold": true }));
    assert_eq!(
        held["brush"], "Smooth",
        "the slot's brush strokes while held"
    );
    let chosen = set(&mut s, json!({ "brush": "Flat Polish", "strength": 0.9 }));
    assert_eq!(chosen["brush"], "Flat Polish");
    assert_eq!(chosen["slots"]["smooth"], "Flat Polish", "Smooth is gone");
    let st = state(&mut s);
    assert_eq!(st["brush.slot.held"], "smooth");
    assert_eq!(st["brush.strength"].as_f64(), Some(f64::from(0.9_f32)));
    let after = slot(&mut s, json!({ "release": true }));
    assert_eq!(after["brush"], "Draw", "the main brush comes back");
    assert_eq!(after["strength"], draw, "Draw's own strength");
    let st = state(&mut s);
    assert_eq!(st["brush.slot.smooth"], "Flat Polish", "until reassigned");
    assert_eq!(st["brush.slot.held"], Value::Null);
    let again = slot(&mut s, json!({ "slot": "smooth", "hold": true }));
    assert_eq!(
        again["strength"].as_f64(),
        Some(f64::from(0.9_f32)),
        "the slot's own"
    );
}

/// Any brush sits in any slot; a brush given with no slot goes where its family fits; stepping
/// goes through the family in `brush.list`'s order and around.
#[test]
fn slots_take_any_brush_and_step_through_their_family() {
    let mut s = Session::new();
    let put = slot(&mut s, json!({ "slot": "mask", "brush": "Clay" }));
    assert_eq!(put["slots"]["mask"], "Clay");
    assert_eq!(put["brush"], "Draw", "the main brush stays");
    let fits = slot(&mut s, json!({ "brush": "Smooth Polish" }));
    assert_eq!(fits["slots"]["smooth"], "Smooth Polish");
    let no_home = s.err("brush.slot", json!({ "brush": "Draw" }));
    assert_eq!(code(no_home.code), "E_ARGS_INVALID", "{}", no_home.message);
    let mut seen = Vec::new();
    for _ in 0..3 {
        let stepped = slot(&mut s, json!({ "slot": "select", "step": true }));
        seen.push(
            stepped["slots"]["select"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
    }
    assert_eq!(seen, ["Clip Curve", "Select Rect", "Clip Curve"]);
    let mask = slot(&mut s, json!({ "slot": "mask", "step": true }));
    assert_eq!(
        mask["slots"]["mask"], "Mask Pen",
        "from Clay, the family's first"
    );
}
