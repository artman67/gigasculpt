//! The words a lit deformer handle says beside the cursor wrap at 280 points in even lines
//! (decision 0062, row 754): the wrap stays, but no line is left holding one short word.

use gs_platform::Modifiers;
use serde_json::json;

use crate::deform_preview::deformer;
use crate::gizmo::gizmo_window;
use crate::rig::{Window, pen_at};

/// How wide the cursor's words grow before they wrap, in points (gs-ui `viewport/cursor/lines.rs`).
const WRAP: f64 = 280.0;

/// The open deformer's cone that turns its angle, as `gizmo.describe` finds it for the
/// window's camera: its outline (tip, then its base's ends) in window pixels.
fn angle_cone(w: &mut Window) -> Vec<[f32; 2]> {
    let d = w.ok("ui.describe", json!({}));
    let v = &d["viewport"];
    let r: [f64; 4] = std::array::from_fn(|i| v["rect"][i].as_f64().unwrap_or_default());
    let size = [(r[2] - r[0]).round() as u32, (r[3] - r[1]).round() as u32];
    let args = json!({ "camera": v["camera"], "viewport": size });
    let shown = w.ok("gizmo.describe", args)["deformer"]["handles"].clone();
    let handles = shown.as_array().cloned().unwrap_or_default();
    let cone = handles
        .iter()
        .find(|h| h["kind"] == "cone" && h["drives"] == "/bend/degrees" && h["pickable"] == true);
    let outline = cone.and_then(|c| c["outline"].as_array().cloned());
    (outline
        .unwrap_or_else(|| panic!("no angle cone in {shown}"))
        .iter())
    .map(|p| [0, 1].map(|i| (r[i] + p[i].as_f64().unwrap_or_default()) as f32))
    .collect()
}

/// Where the 3D view shows the line or ring `label`, in window pixels.
pub(crate) fn line(w: &mut Window, label: &str) -> [f64; 4] {
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let view = panels.iter().find(|p| p["id"] == "viewport").cloned();
    let widgets = view.and_then(|v| v["widgets"].as_array().cloned());
    let found = (widgets.unwrap_or_default().into_iter()).find(|x| x["label"] == label);
    let found = found.unwrap_or_else(|| panic!("no {label:?} beside the cursor"));
    std::array::from_fn(|i| found["rect"][i].as_f64().unwrap_or_default())
}

/// A bend's cone says its row, then its hint, which runs past the wrap: the hint takes two
/// lines, each about half of it, where the greedy wrap left "5°." alone under a line nearly
/// the wrap's width.
#[test]
fn a_lit_cone_s_hint_wraps_in_even_lines_none_wider_than_the_wrap() {
    let mut w = gizmo_window("words-wrap", 1.0, 75.0);
    deformer(&mut w, "open", Some(0.0));
    let cone = angle_cone(&mut w);
    let [a, b] = [cone[1], cone[2]];
    w.pen(pen_at(
        1,
        f32::midpoint(a[0], b[0]),
        f32::midpoint(a[1], b[1]),
        0.0,
    ));
    w.input(&[], Modifiers::default());
    let row = line(&mut w, "Angle  +0");
    let hint = line(&mut w, "The angle, in degrees. Shift snaps to 5°.");
    let (high, width) = (hint[3] - hint[1], hint[2] - hint[0]);
    let lines = (high / (row[3] - row[1])).round();
    assert_eq!(lines, 2.0, "the hint wraps once: {hint:?} under {row:?}");
    assert!(width <= WRAP, "no wider than the wrap: {width}");
    assert!(
        width < 0.7 * WRAP,
        "two even lines, not one and a word: {width}"
    );
}
