//! Strength and Falloff step commands (decision 0052, section 10): `brush.set`'s
//! `strength_step` and `falloff_step` move the brush's value by the session's increments, held
//! to their ranges, and the increments, set by `strength_increment` and `falloff_increment`,
//! are the same for every brush and show in `session.state`.

use gs_testkit::{Session, code};
use serde_json::{Value, json};

fn set(s: &mut Session, args: Value) -> Value {
    s.query("brush.set", args)
}

/// A number as the `f32` it stands for.
fn single(v: &Value) -> f32 {
    v.as_f64().map_or(f32::NAN, |x| x as f32)
}

fn near(v: &Value, want: f32) -> bool {
    (single(v) - want).abs() < 1e-4
}

#[test]
fn strength_and_falloff_step_by_increments_every_brush_shares() {
    let mut s = Session::new();
    let start = set(
        &mut s,
        json!({ "brush": "Draw", "strength": 0.5, "falloff": 0 }),
    );
    assert!(near(&start["strength"], 0.5), "{start}");
    let state = s.query("session.state", json!({}));
    assert!(near(&state["brush.strength_increment"], 0.05), "{state}");
    assert!(near(&state["brush.falloff_increment"], 10.0), "{state}");

    let up = set(&mut s, json!({ "strength_step": 1 }));
    assert!(near(&up["strength"], 0.55), "one increment of 0.05: {up}");
    let down = set(&mut s, json!({ "strength_step": -2 }));
    assert!(near(&down["strength"], 0.45), "{down}");
    let harder = set(&mut s, json!({ "falloff_step": 2 }));
    assert!(
        near(&harder["falloff"], 20.0),
        "two increments of 10: {harder}"
    );

    // A new increment applies to a step in the same call, and is kept for the next.
    let wider = set(
        &mut s,
        json!({ "strength_increment": 0.1, "strength_step": 1 }),
    );
    assert!(near(&wider["strength"], 0.55), "{wider}");
    let again = set(&mut s, json!({ "strength_step": 1 }));
    assert!(
        near(&again["strength"], 0.65),
        "the 0.1 is remembered: {again}"
    );
    set(&mut s, json!({ "falloff_increment": 25 }));

    // The increments belong to the session, not to the brush.
    let clay = set(
        &mut s,
        json!({ "brush": "Clay Build", "strength": 0.2, "falloff": 50 }),
    );
    assert!(near(&clay["strength"], 0.2), "{clay}");
    let clay = set(&mut s, json!({ "strength_step": 1, "falloff_step": -1 }));
    assert!(near(&clay["strength"], 0.3), "{clay}");
    assert!(near(&clay["falloff"], 25.0), "{clay}");
    let state = s.query("session.state", json!({}));
    assert!(near(&state["brush.strength_increment"], 0.1), "{state}");
    assert!(near(&state["brush.falloff_increment"], 25.0), "{state}");

    // Held to their ranges.
    let full = set(&mut s, json!({ "strength_step": 100 }));
    assert!(near(&full["strength"], 1.0), "{full}");
    let widest = set(&mut s, json!({ "falloff_step": -100 }));
    assert!(near(&widest["falloff"], -100.0), "{widest}");

    // A step and a value in one call, or an increment out of range, is refused.
    for args in [
        json!({ "strength": 0.3, "strength_step": 1 }),
        json!({ "falloff": 10, "falloff_step": 1 }),
        json!({ "strength_increment": 0 }),
        json!({ "falloff_increment": 80 }),
    ] {
        let refused = s.err("brush.set", args.clone());
        assert_eq!(code(refused.code), "E_ARGS_INVALID", "{args}");
    }
}
