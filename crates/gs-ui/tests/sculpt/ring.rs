//! The cursor's rings (`interface.cursor-ring`): the Size and Softness rings lie flat on the
//! surface under the pen, a circle on it, seen narrower across where the surface turns away;
//! off the model, or with the setting off, they are plain circles. The mirror rings can be
//! turned off.

use serde_json::json;

use crate::rig::{Window, pen_at, with_sphere};

/// The width and height of the 3D view's widget labelled `label`, in window pixels, and how
/// many there are.
fn rings(w: &mut Window, label: &str) -> Vec<[f64; 2]> {
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let view = panels.iter().find(|p| p["id"] == "viewport").cloned();
    let widgets = view.and_then(|v| v["widgets"].as_array().cloned());
    (widgets.unwrap_or_default().iter())
        .filter(|x| x["label"] == label)
        .map(|x| {
            let r = |i: usize| x["rect"][i].as_f64().unwrap_or_default();
            [r(2) - r(0), r(3) - r(1)]
        })
        .collect()
}

/// The Size ring's width and height with the pen hovering at `at`.
fn size_ring(w: &mut Window, t: u64, at: [f32; 2]) -> [f64; 2] {
    w.pen(pen_at(t, at[0], at[1], 0.0));
    let found = rings(w, "Size ring");
    assert_eq!(found.len(), 1, "one Size ring at {at:?}");
    found[0]
}

#[test]
fn the_size_ring_lies_flat_on_a_tilted_surface() {
    // Taller than the rig's window, whose 3D view the shelf's three rows shorten: the sphere
    // fills a view this tall wide enough for the ring to lie on its side.
    let mut w = Window::sized(None, gs_ui::Keymap::default(), [640, 600], 1.0);
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let rect = w.view_rect();
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    // The sphere's right edge on screen, level with its middle.
    let on = |w: &mut Window, x: f32| !w.picked(x, cy)["hit"].is_null();
    let mut edge = cx;
    while on(&mut w, edge + 2.0) {
        edge += 2.0;
    }
    assert!(edge > cx + 40.0, "the sphere shows: its edge at {edge}");
    // Over its middle the surface faces the eye: a circle the brush's size, 64 pixels out.
    let [width, height] = size_ring(&mut w, 1, [cx, cy]);
    assert!(
        (width - 128.0).abs() < 3.0 && (height - 128.0).abs() < 3.0,
        "a circle 128 pixels across: {width} by {height}"
    );
    // Near its right edge the surface turns away to the right: the ring lies on it, narrower
    // across than up and down.
    let tilted = [cx + 0.85 * (edge - cx), cy];
    let [width, height] = size_ring(&mut w, 2, tilted);
    assert!(
        width < 0.75 * height,
        "flat on the surface: {width} by {height}"
    );
    let falloff = rings(&mut w, "Softness ring");
    assert!(
        falloff.len() == 1 && falloff[0][0] < 0.75 * falloff[0][1],
        "the Softness ring too: {falloff:?}"
    );
    // Off the model it is a plain circle.
    let [width, height] = size_ring(&mut w, 3, [rect[0] + 70.0, rect[1] + 70.0]);
    assert!(
        (width - height).abs() < 1.0 && (width - 128.0).abs() < 3.0,
        "off the model: {width} by {height}"
    );
    // With the setting off, a plain circle on the surface too.
    w.ok("ui.settings", json!({ "ring_on_surface": false }));
    let [width, height] = size_ring(&mut w, 4, tilted);
    assert!(
        (width - height).abs() < 1.0,
        "the setting off: {width} by {height}"
    );
}

#[test]
fn the_mirror_rings_can_be_turned_off() {
    let (mut w, rect) = with_sphere(None);
    let x = f32::midpoint(rect[0], rect[2]) + 40.0;
    let y = f32::midpoint(rect[1], rect[3]) - 25.0;
    w.ok(
        "brush.set",
        json!({ "symmetry": { "mirror": { "x": true } } }),
    );
    w.pen(pen_at(1, x, y, 0.0));
    assert_eq!(rings(&mut w, "Mirror ring").len(), 1, "on to start");
    w.ok("ui.settings", json!({ "mirror_rings": false }));
    w.pen(pen_at(2, x, y, 0.0));
    assert_eq!(rings(&mut w, "Mirror ring").len(), 0, "off");
}
