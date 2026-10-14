//! What the 3D view shows and sends to the GPU: the brush circle at the brush's size, a part
//! uploaded whole when it is new and only where a stroke changed it afterwards, and parts with
//! nothing to draw left out.

use gs_ui::{Keymap, Render};
use serde_json::json;

use crate::rig::{Window, pen_at};

/// Bytes the window sent to the GPU after the last call.
fn uploaded(w: &Window) -> u64 {
    w.host.lock().render.as_ref().map_or(0, Render::uploaded)
}

#[test]
fn the_brush_circle_has_the_brush_size_on_screen() {
    // Two pixels per point: a 64 pixel brush is a circle 64 pixels across the middle, not 64
    // points.
    // At two pixels per point the small window is 320 points wide, too narrow beside the trays
    // for the circle and its label.
    let mut w = Window::sized(None, Keymap::default(), [2200, 1000], 2.0);
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let rect = w.view_rect();
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    w.pen(pen_at(0, cx, cy, 0.0));
    let width = (rect[2] - rect[0]).round() as usize;
    let (left, top) = (rect[0].round(), rect[1].round());
    // The circle's warm tint, left of the pen (its label is on the right) and beyond the
    // Falloff ring, which sits at half the size; the sphere is gray and the hover ring cool
    // blue.
    let warm: Vec<f32> = (w.view_pixels().chunks(4).enumerate())
        .filter(|(_, p)| p[0] > 150 && i32::from(p[0]) - i32::from(p[2]) > 40)
        .map(|(i, _)| {
            let x = left + (i % width) as f32 + 0.5 - cx;
            let y = top + (i / width) as f32 + 0.5 - cy;
            (x, (x * x + y * y).sqrt())
        })
        .filter(|&(x, d)| x < 0.0 && d > 48.0)
        .map(|(_, d)| d)
        .collect();
    assert!(warm.len() > 100, "the circle shows: {} pixels", warm.len());
    let off = warm.iter().map(|d| (d - 64.0).abs()).fold(0.0, f32::max);
    assert!(
        off < 3.0,
        "every pixel of it is 64 pixels out, give or take {off}"
    );
    let label = w.brush_label();
    assert!(
        label
            .as_ref()
            .is_some_and(|(text, r)| text == "Draw  64 px  40%" && r[0] > f64::from(cx)),
        "the brush's name, size and strength just right of the pen: {label:?}"
    );
}

#[test]
fn a_part_is_uploaded_whole_once_then_only_where_it_changed() {
    let mut w = Window::new(None);
    // Several nodes of the engine's size.
    w.ok("primitive.sphere", json!({ "subdivisions": 100 }));
    let whole = uploaded(&w);
    assert!(whole > 0, "the new part went to the GPU");
    w.ok("mesh.hash", json!({}));
    assert_eq!(uploaded(&w), 0, "a call that changes nothing sends nothing");
    let small = json!({ "brush": "Draw", "size": 0.05, "samples": [{ "position": [0, 0, 1] }] });
    w.ok("brush.stroke", small);
    let stroke = uploaded(&w);
    assert!(
        stroke > 0 && stroke * 4 < whole,
        "a small stroke sends its nodes: {stroke} of {whole} bytes"
    );
}

#[test]
fn hiding_showing_reordering_and_moving_a_part_send_nothing() {
    // Decision 0029: a property change keeps the part's upload identity, and the window keeps
    // a hidden part's GPU copy without drawing it.
    let mut w = Window::new(None);
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    w.ok("primitive.cube", json!({}));
    // The active part always shows (decision 0033), so the cube hidden here must not be it.
    w.ok("part.select", json!({ "part": "sphere" }));
    let parts = |w: &mut Window| w.ok("ui.describe", json!({}))["viewport"]["parts"].clone();
    let calls = [
        (
            "part.set_visible",
            json!({ "parts": ["cube"], "visible": false }),
        ),
        ("part.reorder", json!({ "part": "cube", "to": "top" })),
        (
            "part.set_visible",
            json!({ "parts": ["cube"], "visible": true }),
        ),
        (
            "transform.apply",
            json!({ "part": "cube", "op": { "rotate": { "axis": "y", "degrees": 30 } } }),
        ),
    ];
    let mut drawn = Vec::new();
    for (command, args) in calls {
        w.ok(command, args);
        assert_eq!(uploaded(&w), 0, "{command} sends nothing");
        drawn.push(parts(&mut w));
    }
    let expected = [
        json!(["sphere"]),
        json!(["sphere"]),
        json!(["cube", "sphere"]),
        json!(["cube", "sphere"]),
    ];
    assert_eq!(drawn, expected, "the view leaves out the hidden part");
    let mirrored = json!({
        "part": "cube",
        "op": { "move": [0.2, 0.0, 0.0] },
        "symmetry": { "mirror": { "x": true } },
    });
    w.ok("transform.apply", mirrored);
    assert!(uploaded(&w) > 0, "a move of the points sends them");
}

#[test]
fn a_part_with_nothing_to_draw_is_left_out() {
    let mut w = Window::new(None);
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let empty = json!({ "name": "nothing", "mesh": { "positions": [], "faces": [] } });
    w.ok("part.create", empty);
    let d = w.ok("ui.describe", json!({}));
    assert_eq!(d["viewport"]["parts"], json!(["sphere"]));
}
