//! The mouse as a hovering pen (decision 0025 section 2, amended 2026-10-12, Win-feel; catalog
//! `core.pen-input`): a mouse move with no button held is a hovering pen sample, so the mouse
//! shows the brush circle and its text as a hovering pen does, stays in range when its button
//! lets go, and leaves range with the window. Beside a pen in range, and for a moment after the
//! pen's last sample, mouse motion makes no second cursor.

use gs_platform::{Button, Input, Modifiers};
use gs_protocol::PenSample;
use gs_ui::{MousePen, split_pen};
use serde_json::{Value, json};

use crate::rig::{Ui, center};

const MS: u64 = 1_000_000;

/// A sphere to hover over, and a spot on it.
fn model() -> (Ui, [f64; 2]) {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let [x, y] = ui.middle();
    (ui, [x - 20.0, y + 10.0])
}

/// One pump of the window's input at `now_ns`, as `app.rs` runs it: the mouse made a pen where
/// it is one (`MousePen`), the samples to `pen.input`, the rest to the studio.
fn pump(ui: &mut Ui, mouse: &mut MousePen, inputs: &[Input], now_ns: u64) {
    let (samples, others) = split_pen(&mouse.convert(inputs, now_ns));
    if !samples.is_empty() {
        ui.ok("pen.input", json!({ "samples": samples }));
    }
    ui.input(&others, Modifiers::default());
}

/// What the 3D view draws: each widget's label and rect.
fn drawn(ui: &mut Ui) -> Vec<Value> {
    (ui.widgets("viewport").iter())
        .map(|w| json!([w["label"], w["rect"]]))
        .collect()
}

/// The centers of the Size rings the 3D view draws.
fn rings(ui: &mut Ui) -> Vec<[f64; 2]> {
    (ui.widgets("viewport").iter())
        .filter(|w| w["label"] == "Size ring")
        .map(|w| center(&w["rect"]))
        .collect()
}

/// Whether there is one Size ring, around `at` (a ring laid on a slanted surface is a little
/// off its spot).
fn one_ring_at(ui: &mut Ui, at: [f64; 2]) -> bool {
    let found = rings(ui);
    found.len() == 1 && libm::hypot(found[0][0] - at[0], found[0][1] - at[1]) < 4.0
}

#[test]
fn a_bare_mouse_move_shows_the_brush_as_a_hovering_pen_does() {
    let (mut pen, at) = model();
    let hover = PenSample::hover(1_000 * MS, at[0], at[1]);
    pen.ok("pen.input", json!({ "samples": [hover] }));
    pen.settle();
    let by_pen = drawn(&mut pen);
    assert!(
        by_pen.iter().any(|w| w[0] == "Size ring"),
        "a hovering pen shows the circle: {by_pen:?}"
    );

    let (mut ui, _) = model();
    let mut mouse = MousePen::default();
    let moved = Input::MouseMove { x: at[0], y: at[1] };
    pump(&mut ui, &mut mouse, &[moved], 1_000 * MS);
    assert_eq!(drawn(&mut ui), by_pen, "the mouse shows what the pen shows");
    let cursor = ui.host.lock().shell.cursor.pen;
    assert!(
        cursor.is_some_and(|c| c.in_proximity && !c.contact && c.pressure == 0.0),
        "a hover, with no pressure: {cursor:?}"
    );

    // A click: the mouse stays in range, hovering where its button let go.
    let button = |down| Input::MouseButton {
        button: Button::Left,
        down,
        x: at[0],
        y: at[1],
    };
    pump(&mut ui, &mut mouse, &[button(true)], 1_010 * MS);
    pump(&mut ui, &mut mouse, &[button(false)], 1_020 * MS);
    assert!(
        one_ring_at(&mut ui, at),
        "still hovering: {:?}",
        rings(&mut ui)
    );

    // Leaving the window ends the hover.
    pump(&mut ui, &mut mouse, &[Input::PointerGone], 1_030 * MS);
    assert!(rings(&mut ui).is_empty(), "gone with the pointer");
}

#[test]
fn a_mouse_move_beside_a_pen_in_range_adds_no_second_cursor() {
    let (mut ui, at) = model();
    let away = [at[0] + 60.0, at[1] - 40.0];
    let mut mouse = MousePen::default();
    let pen = |ms: u64, near: bool| {
        Input::Pen(PenSample {
            in_proximity: near,
            ..PenSample::hover(ms * MS, at[0], at[1])
        })
    };
    let moved = Input::MouseMove {
        x: away[0],
        y: away[1],
    };
    pump(&mut ui, &mut mouse, &[pen(1_000, true)], 1_000 * MS);
    assert!(one_ring_at(&mut ui, at), "the pen's: {:?}", rings(&mut ui));
    // The system pointer the pen moves, or a mouse beside it: the pen's cursor stays the only
    // one.
    pump(
        &mut ui,
        &mut mouse,
        std::slice::from_ref(&moved),
        1_100 * MS,
    );
    assert!(one_ring_at(&mut ui, at), "one cursor: {:?}", rings(&mut ui));
    // The pen leaves range; just after, mouse motion still makes no cursor of its own.
    pump(&mut ui, &mut mouse, &[pen(1_200, false)], 1_200 * MS);
    pump(
        &mut ui,
        &mut mouse,
        std::slice::from_ref(&moved),
        1_300 * MS,
    );
    assert!(rings(&mut ui).is_empty(), "just after the pen");
    // Half a second on, the mouse hovers as a pen.
    pump(&mut ui, &mut mouse, &[moved], 1_800 * MS);
    assert!(
        one_ring_at(&mut ui, away),
        "the mouse's, once the pen has gone: {:?}",
        rings(&mut ui)
    );
}
