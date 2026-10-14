//! A stroke that changes nothing says why, so an agent can correct it: how far its nearest
//! sample was from the surface, or that it reached the surface and still changed nothing.

use gs_testkit::code;
use serde_json::json;

use crate::{line, sphere_session, stroke};

/// The number after `marker` in `text`.
fn number_after(text: &str, marker: &str) -> f32 {
    let rest = text
        .split(marker)
        .nth(1)
        .unwrap_or_else(|| panic!("no '{marker}' in: {text}"));
    let digits: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.parse().unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[test]
fn a_stroke_off_the_surface_says_how_far_it_was() {
    let mut s = sphere_session(None);
    let before = s.hash();
    // Two units in front of the sphere, which reaches z = 1.
    let off = line([-0.2, 0.0, 3.0], [0.2, 0.0, 3.0], 5);
    let done = s.ok(
        "brush.stroke",
        stroke("Draw", &off, &json!({ "size": 0.1 })),
    );
    assert_eq!(s.hash(), before, "nothing changed");
    assert!(done.history_id.is_none(), "and no step was made");
    let effect = done
        .effect
        .unwrap_or_else(|| panic!("an edit reports its effect"));
    let reason = effect.noop_reason.unwrap_or_default();
    assert!(reason.contains("never reached sphere"), "{reason}");
    let gap = number_after(&reason, "its nearest sample was ");
    assert!(
        (gap - 2.0).abs() < 1e-3,
        "the nearest sample was 2 units off, not {gap}"
    );
}

#[test]
fn a_stroke_that_reaches_but_changes_nothing_says_so() {
    let mut s = sphere_session(None);
    let across = line([-0.3, 0.0, 0.95], [0.3, 0.0, 0.95], 5);
    let done = s.ok(
        "brush.stroke",
        stroke("Draw", &across, &json!({ "strength": 0 })),
    );
    let reason = done.effect.and_then(|e| e.noop_reason).unwrap_or_default();
    assert!(
        reason.contains("reached sphere but changed nothing"),
        "{reason}"
    );
}

#[test]
fn bad_samples_are_refused_where_they_are() {
    let mut s = sphere_session(None);
    let mut args = stroke("Draw", &[[0.0, 0.0, 1.0], [0.1, 0.0, 1.0]], &json!({}));
    args["samples"][1]["position"] = json!([0.1, 0.0]);
    let err = s.err("brush.stroke", args);
    assert_eq!(
        (code(err.code).as_str(), err.path.as_deref()),
        ("E_ARGS_INVALID", Some("/samples/1/position")),
        "{}",
        err.message
    );
}
