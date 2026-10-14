//! Gesture strokes off the model in the window (decision 0017 as amended 2026-10-12, items 17
//! to 22): a Clip Curve drag from empty canvas cuts and leaves the view where it was, where a
//! Draw drag there turns the view; Ctrl under a gesture brush draws it from empty canvas instead
//! of working the mask; Clip Curve's shadow side lies on the side the cut takes and flips with
//! Alt; a held slot brush's curve stroke keeps its path and bends through an Alt tap.

use gs_platform::Modifiers;
use serde_json::json;

use crate::background::ran_since;
use crate::held::keys;
use crate::region;
use crate::rig::{Window, log_path, pen_at, with_sphere};

/// `main`'s camera as JSON text (equal text is equal bits).
fn camera(w: &mut Window) -> String {
    w.ok("view.get", json!({ "view": "main" }))["camera"].to_string()
}

/// Whether the window's camera shows a part at (`x`, `y`).
fn hits(w: &mut Window, [x, y]: [f32; 2]) -> bool {
    !w.picked(x, y)["hit"].is_null()
}

/// A hover at `from`, the tip touching there and dragged to `to` in six steps, a frame per
/// sample with `held` held, then the lift.
fn drag(w: &mut Window, from: [f32; 2], to: [f32; 2], held: Modifiers) {
    w.input(&[], held);
    w.pen(pen_at(0, from[0], from[1], 0.0));
    w.input(&[], held);
    for k in 0..=6_u8 {
        let f = f32::from(k) / 6.0;
        let [x, y] = [0, 1].map(|i| from[i] + (to[i] - from[i]) * f);
        w.pen(pen_at(4 + 4 * u64::from(k), x, y, 0.5));
        w.input(&[], held);
    }
    w.pen(pen_at(40, to[0], to[1], 0.0));
    w.input(&[], held);
}

/// The brushes of the strokes logged after the log's first `from` lines.
fn strokes_since(w: &Window, from: usize) -> Vec<String> {
    (ran_since(w, from).into_iter())
        .filter(|l| l.0 == "brush.stroke")
        .map(|l| l.1["brush"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Where the 3D view draws `label` now, in window pixels, if it does.
fn drawn(w: &mut Window, label: &str) -> Option<[f64; 4]> {
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let view = panels.iter().find(|p| p["id"] == "viewport")?.clone();
    let widgets = view["widgets"].as_array().cloned().unwrap_or_default();
    let found = widgets.iter().find(|x| x["label"] == label)?;
    Some(std::array::from_fn(|i| {
        found["rect"][i].as_f64().unwrap_or_default()
    }))
}

#[test]
fn a_clip_curve_drag_from_empty_canvas_cuts_and_a_draw_drag_there_turns_the_view() {
    let (mut w, rect) = with_sphere(Some(log_path("off-model")));
    let [_, cy] = region::middle(rect.map(f64::from)).map(|v| v as f32);
    let (from, to) = ([rect[0] + 20.0, cy - 20.0], [rect[2] - 20.0, cy - 20.0]);
    assert!(
        !hits(&mut w, from) && !hits(&mut w, to),
        "both ends off the sphere"
    );
    w.ok("brush.set", json!({ "brush": "Clip Curve" }));
    let (view, hash, start) = (camera(&mut w), w.hash(), w.logged().len());
    drag(&mut w, from, to, keys(false, false, false));
    assert_eq!(
        strokes_since(&w, start),
        ["Clip Curve"],
        "one Clip Curve stroke"
    );
    assert_ne!(w.hash(), hash, "it cut the sphere");
    assert_eq!(camera(&mut w), view, "and the view stayed");

    w.ok("brush.set", json!({ "brush": "Draw" }));
    let (hash, start) = (w.hash(), w.logged().len());
    drag(&mut w, from, to, keys(false, false, false));
    assert!(
        strokes_since(&w, start).is_empty(),
        "a Draw drag there strokes nothing"
    );
    assert_eq!(w.hash(), hash, "and changes nothing");
    assert_ne!(camera(&mut w), view, "it turns the view");
}

#[test]
fn ctrl_under_a_gesture_brush_draws_it_from_empty_canvas_instead_of_working_the_mask() {
    let (mut w, rect) = with_sphere(Some(log_path("off-model-ctrl")));
    w.ok(
        "brush.slot",
        json!({ "slot": "mask", "brush": "Select Rect" }),
    );
    let ctrl = keys(false, true, false);
    let corner = [rect[0] + 30.0, rect[3] - 30.0];
    let middle = region::middle(rect.map(f64::from)).map(|v| v as f32);
    assert!(!hits(&mut w, corner), "the corner is empty canvas");
    let (view, hash, start) = (camera(&mut w), w.hash(), w.logged().len());
    drag(&mut w, corner, middle, ctrl);
    let ran: Vec<String> = ran_since(&w, start).into_iter().map(|l| l.0).collect();
    assert_eq!(
        ran,
        ["brush.stroke"],
        "the drag is a stroke and nothing else"
    );
    assert_eq!(
        strokes_since(&w, start),
        ["Select Rect"],
        "with Ctrl's brush"
    );
    let shown = w.hash();
    assert_ne!(shown, hash, "it hid what lies outside the rectangle");
    assert_eq!(camera(&mut w), view, "and the view stayed");

    // A click there is a rectangle with no area: it changes nothing, and the mask is not
    // inverted.
    let start = w.logged().len();
    drag(&mut w, corner, corner, ctrl);
    let ran: Vec<String> = ran_since(&w, start).into_iter().map(|l| l.0).collect();
    assert!(!ran.iter().any(|c| c.starts_with("mask.")), "{ran:?}");
    assert_eq!(w.hash(), shown, "nothing changed");
}

#[test]
fn the_shadow_side_lies_where_the_clip_cuts_and_flips_with_alt() {
    let (mut w, rect) = with_sphere(Some(log_path("off-model-shadow")));
    w.ok("brush.set", json!({ "brush": "Clip Curve" }));
    let [cx, cy] = region::middle(rect.map(f64::from)).map(|v| v as f32);
    let (above, below) = ([cx, cy - 30.0], [cx, cy + 30.0]);
    assert!(
        hits(&mut w, above) && hits(&mut w, below),
        "the sphere fills both"
    );
    let none = keys(false, false, false);
    // Rightward across the sphere's middle from empty canvas, the pen held down.
    let x = |k: u8| rect[0] + 20.0 + (rect[2] - rect[0] - 40.0) * f32::from(k) / 6.0;
    w.pen(pen_at(0, x(0), cy, 0.0));
    w.input(&[], none);
    for k in 0..=6_u8 {
        w.pen(pen_at(4 + 4 * u64::from(k), x(k), cy, 0.5));
        w.input(&[], none);
    }
    // Where the band lies from the path's middle, down positive.
    let side = |w: &mut Window| {
        let path = drawn(w, "Curve path").unwrap_or_else(|| panic!("no path drawn"));
        let band = drawn(w, "Shadow side").unwrap_or_else(|| panic!("no shadow side drawn"));
        (band[1] + band[3] - path[1] - path[3]) / 2.0
    };
    assert!(
        side(&mut w) < -5.0,
        "above the path, left of its rightward run"
    );
    w.input(&[], keys(false, false, true));
    assert!(side(&mut w) > 5.0, "Alt down flips it below");
    w.input(&[], none);
    assert!(side(&mut w) < -5.0, "and back with Alt up");
    w.pen(pen_at(60, x(6), cy, 0.0));
    w.input(&[], none);
    assert_eq!(drawn(&mut w, "Shadow side"), None, "gone with the lift");
    // The cut took the side the band showed: seen from the camera the clipped part lies flat
    // along the path, so above it the sphere is gone and below it stays.
    assert!(!hits(&mut w, above), "above the path the sphere is clipped");
    assert!(hits(&mut w, below), "below it stays");
}

#[test]
fn a_held_slot_brush_s_curve_stroke_bends_through_an_alt_tap() {
    let (mut w, rect) = with_sphere(Some(log_path("off-model-slot")));
    w.ok(
        "brush.slot",
        json!({ "slot": "select", "brush": "Clip Curve" }),
    );
    let [cx, cy] = region::middle(rect.map(f64::from)).map(|v| v as f32);
    let select = keys(true, true, false);
    let at = |k: u8| {
        let y = if k.is_multiple_of(2) { cy } else { cy + 10.0 };
        pen_at(
            4 + 4 * u64::from(k),
            cx - 80.0 + 25.0 * f32::from(k),
            y,
            0.5,
        )
    };
    w.input(&[], select);
    let mut hover = at(0);
    (hover.contact, hover.pressure) = (false, 0.0);
    w.pen(hover);
    w.input(&[], select);
    for k in 0..=6_u8 {
        if k == 3 {
            w.input(&[], keys(true, true, true));
            w.input(&[], select);
        }
        w.pen(at(k));
        w.input(&[], select);
    }
    assert!(drawn(&mut w, "Curve path").is_some(), "the path is drawn");
    let mut up = at(6);
    (up.contact, up.pressure) = (false, 0.0);
    w.pen(up);
    w.input(&[], select);
    w.input(&[], keys(false, false, false));
    let logged = w.logged();
    let line = logged.iter().rev().find(|l| l.0 == "brush.stroke");
    let line = line.unwrap_or_else(|| panic!("no stroke logged"));
    assert_eq!(line.1["brush"], json!("Clip Curve"), "the slot's brush");
    let samples = line.1["samples"].as_array().cloned().unwrap_or_default();
    let points: Vec<(usize, &str)> = (samples.iter().enumerate())
        .filter_map(|(i, s)| s["point"].as_str().map(|p| (i, p)))
        .collect();
    assert_eq!(
        points,
        [(2, "soft")],
        "the tap's point, on the sample before it"
    );
}
