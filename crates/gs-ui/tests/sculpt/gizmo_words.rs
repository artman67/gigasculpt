//! With the gizmo as the tool, the cursor's words are the gizmo's (decision 0062, row 816):
//! Reach, the brush's Size a soft drag reaches to, and the gizmo's own Softness (its falloff),
//! with its ring, not the brush's name, strength, Softness or symmetry, which a gizmo drag does
//! not use.

use gs_platform::Modifiers;
use serde_json::json;

use crate::gizmo::gizmo_window;
use crate::rig::{Window, pen_at};
use crate::words_wrap::line;

/// The width of the 3D view's widget labelled `label`, in window pixels.
fn width(w: &mut Window, label: &str) -> f64 {
    let r = line(w, label);
    r[2] - r[0]
}

#[test]
fn with_the_gizmo_the_cursor_names_reach_and_its_falloff_not_the_brush() {
    let mut w = gizmo_window("gizmo-words", 1.0, 75.0);
    w.ok("ui.gizmo", json!({ "falloff": 40 }));
    w.ok("brush.set", json!({ "falloff": -30 }));
    // Off the model and the gizmo's handles, where the rings are circles.
    let r = w.view_rect();
    let at = [r[0] + 0.15 * (r[2] - r[0]), f32::midpoint(r[1], r[3])];
    w.pen(pen_at(1, at[0], at[1], 0.0));
    w.input(&[], Modifiers::default());
    let lines = w.cursor_lines();
    let has = |l: &str| lines.iter().any(|x| x == l);
    assert!(has("Reach 64 px") && has("Softness +40"), "{lines:?}");
    let brush = ["Draw", "%", "-30", "symmetry", "Mirror"];
    assert!(
        !(lines.iter()).any(|l| brush.iter().any(|b| l.contains(b))),
        "no brush setting: {lines:?}"
    );
    // The Softness ring at +40 is where gs-core's curve reaches one half: 0.5^(2^0.8) of the
    // Size ring, about 0.30, where the brush's -30 would put it near 0.63.
    let inside = width(&mut w, "Softness ring") / width(&mut w, "Size ring");
    let want = libm::pow(0.5, libm::pow(2.0, 0.8));
    assert!(
        (inside - want).abs() < 0.02,
        "the gizmo's: {inside} for {want}"
    );
}
