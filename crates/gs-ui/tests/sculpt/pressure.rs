//! Pen pressure through the window (catalog `brush-settings.tablet-pressure` and
//! `interface.pen-settings`, decision 0036 section 3 as amended): a sample sent without its
//! pressure presses fully, as a `brush.stroke` sample without one does (research 27's R17), and
//! while the tip touches, the Size ring is the size the dab takes at the pen's pressure, through
//! the size curve and Size Sensitivity.

use gs_protocol::PenSample;
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::rig::{Window, log_path, pen_at, with_sphere};

#[test]
fn a_pen_sample_without_pressure_strokes_as_a_brush_stroke_sample_without_one() {
    let (mut w, rect) = with_sphere(Some(log_path("no-pressure")));
    let (cx, cy) = (
        f64::from(f32::midpoint(rect[0], rect[2])),
        f64::from(f32::midpoint(rect[1], rect[3])),
    );
    // A hover, five touches and the lift, as an agent writes them: no pressure anywhere.
    let at = |k: u64, contact: bool| {
        let x = cx - 40.0 + 16.0 * k as f64;
        json!({ "t_ns": k * 4_000_000, "x": x, "y": cy, "contact": contact })
    };
    let before = w.hash();
    for k in 0..7 {
        w.ok(
            "pen.input",
            json!({ "samples": [at(k, (1..6).contains(&k))] }),
        );
    }
    assert_ne!(w.hash(), before, "it sculpted");
    let logged = w.logged().into_iter().rfind(|l| l.0 == "brush.stroke");
    let stroke = logged.map_or_else(|| panic!("a stroke"), |l| l.1);
    let samples = stroke["samples"].as_array().cloned().unwrap_or_default();
    let pressures: Vec<&Value> = samples.iter().map(|s| &s["pressure"]).collect();
    assert_eq!(pressures, vec![&json!(1.0); 5], "full pressure: {stroke}");
    // The same call with every pressure left out, run without a window.
    let mut bare = stroke.clone();
    for s in bare["samples"].as_array_mut().into_iter().flatten() {
        s.as_object_mut().map(|m| m.remove("pressure"));
    }
    let mut headless = Session::new();
    headless.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    headless.ok("brush.stroke", bare);
    assert_eq!(headless.hash(), w.hash(), "both press fully");
}

/// The Size ring's width, in window pixels, with the pen at `at` at `pressure` (hovering at 0).
fn ring_width(w: &mut Window, t: u64, at: [f32; 2], pressure: f32) -> f64 {
    w.pen(pen_at(t, at[0], at[1], pressure));
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let view = panels.iter().find(|p| p["id"] == "viewport").cloned();
    let widgets = view.and_then(|v| v["widgets"].as_array().cloned());
    let rings: Vec<f64> = (widgets.unwrap_or_default().iter())
        .filter(|x| x["label"] == "Size ring")
        .map(|x| {
            x["rect"][2].as_f64().unwrap_or_default() - x["rect"][0].as_f64().unwrap_or_default()
        })
        .collect();
    assert_eq!(rings.len(), 1, "one Size ring");
    rings[0]
}

#[test]
fn the_size_ring_takes_the_size_the_pressure_gives() {
    let (mut w, rect) = with_sphere(None);
    // Plain circles, so the width is the size.
    w.ok("ui.settings", json!({ "ring_on_surface": false }));
    let at = [
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    ];
    // Draw's size is 64 pixels: 128 across, at any pressure while no size curve is on.
    let near = |w: f64, want: f64| (w - want).abs() < 2.0;
    let width = ring_width(&mut w, 0, at, 0.0);
    assert!(near(width, 128.0), "hovering: {width}");
    let width = ring_width(&mut w, 4, at, 0.3);
    assert!(near(width, 128.0), "no size curve: {width}");
    ring_width(&mut w, 8, at, 0.0);
    // The straight size curve: the ring follows the pressure.
    w.ok(
        "brush.set",
        json!({ "global_pressure": { "size": [[0, 0], [1, 1]] } }),
    );
    let width = ring_width(&mut w, 12, at, 0.3);
    assert!(near(width, 128.0 * 0.3), "at 0.3: {width}");
    let width = ring_width(&mut w, 16, at, 0.9);
    assert!(near(width, 128.0 * 0.9), "at 0.9: {width}");
    ring_width(&mut w, 20, at, 0.0);
    let width = ring_width(&mut w, 24, at, 0.0);
    assert!(near(width, 128.0), "hovering again: {width}");
    // Size Sensitivity -50: the ring shrinks as the pressure rises, to half at full.
    w.ok("pen.set", json!({ "size_sensitivity": -50 }));
    let light = ring_width(&mut w, 28, at, 0.3);
    let firm = ring_width(&mut w, 32, at, 0.9);
    assert!(near(light, 128.0 * 0.85), "at 0.3: {light}");
    assert!(near(firm, 128.0 * 0.55), "at 0.9: {firm}");
    let lift = PenSample {
        contact: false,
        ..pen_at(36, at[0], at[1], 0.0)
    };
    w.pen(lift);
}
