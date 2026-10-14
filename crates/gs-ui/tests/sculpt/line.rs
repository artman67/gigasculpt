//! Shift held after a stroke's press holds it to a straight line: a dab stroke the way it first
//! moved (`stroke.freehand`), a curve from the press in 45 degree steps (`stroke.curve-line`),
//! a slot's stroke as the session brush's.

use gs_platform::Modifiers;
use serde_json::json;

use crate::rig::{Window, log_path, pen_at, with_sphere};

/// The positions of the `n`th logged brush stroke's samples, in the view's pixels.
fn positions(w: &Window, n: usize) -> Vec<[f64; 2]> {
    let strokes: Vec<_> = (w.logged().into_iter())
        .filter(|l| l.0 == "brush.stroke")
        .map(|l| l.1)
        .collect();
    let samples = strokes.get(n).map(|s| s["samples"].clone());
    (samples
        .as_ref()
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten())
    .map(|s| {
        let p = |i: usize| s["position"][i].as_f64().unwrap_or_default();
        [p(0), p(1)]
    })
    .collect()
}

/// A stroke from `from` through `moves`, in window pixels, with the keys held changed to each
/// `(k, held)` of `keys` before the move `k`, and let go once the pen lifts.
fn stroke(w: &mut Window, from: [f32; 2], moves: &[[f32; 2]], keys: &[(usize, Modifiers)]) {
    w.pen(pen_at(0, from[0], from[1], 0.0));
    w.pen(pen_at(4, from[0], from[1], 0.5));
    let mut t = 8;
    for (k, m) in moves.iter().enumerate() {
        for (_, held) in keys.iter().filter(|c| c.0 == k) {
            w.input(&[], *held);
        }
        w.pen(pen_at(t, m[0], m[1], 0.5));
        t += 4;
    }
    let [x, y] = moves.last().copied().unwrap_or(from);
    w.pen(pen_at(t, x, y, 0.0));
    w.input(&[], Modifiers::default());
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::default()
    }
}

fn ctrl(shift: bool) -> Modifiers {
    Modifiers {
        ctrl: true,
        shift,
        ..Modifiers::default()
    }
}

/// The brush the `n`th logged brush stroke used.
fn brush(w: &Window, n: usize) -> serde_json::Value {
    let logged = w.logged();
    let mut strokes = logged.iter().filter(|l| l.0 == "brush.stroke");
    strokes
        .nth(n)
        .map(|l| l.1["brush"].clone())
        .unwrap_or_default()
}

#[test]
fn shift_after_the_press_holds_a_dab_stroke_to_its_first_direction() {
    let (mut w, rect) = with_sphere(Some(log_path("shift-line")));
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    // Five moves to the right, Shift with the fifth, then on down and to the right.
    let moves: Vec<[f32; 2]> = (1..=10_u8)
        .map(|k| {
            let k = f32::from(k);
            [cx - 60.0 + 10.0 * k, cy + 8.0 * (k - 5.0).max(0.0)]
        })
        .collect();
    stroke(&mut w, [cx - 60.0, cy], &moves, &[(4, shift())]);
    let line = positions(&w, 0);
    assert_eq!(line.len(), 11, "the press and every move: {line:?}");
    assert!(
        line.iter().all(|p| (p[1] - line[0][1]).abs() < 1e-3),
        "every sample on the line the stroke first took: {line:?}"
    );
    assert!(
        line[10][0] > line[4][0] + 50.0,
        "it runs on along the line: {line:?}"
    );
    assert_eq!(
        brush(&w, 0),
        json!("Draw"),
        "Shift after the press keeps the stroke's brush"
    );
    // Without Shift the same path bends.
    stroke(&mut w, [cx - 60.0, cy], &moves, &[]);
    let bent = positions(&w, 1);
    assert!(
        bent.len() == 11 && bent[10][1] > bent[0][1] + 30.0,
        "without Shift the stroke follows the pen: {bent:?}"
    );
}

#[test]
fn shift_after_the_press_holds_a_curve_to_45_degree_steps() {
    let (mut w, rect) = with_sphere(Some(log_path("shift-curve")));
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    w.ok("brush.set", json!({ "brush": "Clay Line" }));
    // Down and to the right at about 40 degrees, wobbling; Shift from the third move.
    let from = [cx - 50.0, cy - 40.0];
    let moves: Vec<[f32; 2]> = (1..=8_u8)
        .map(|k| {
            let (k, wobble) = (f32::from(k), if k % 2 == 0 { 3.0 } else { -3.0 });
            [from[0] + 10.0 * k, from[1] + 8.4 * k + wobble]
        })
        .collect();
    stroke(&mut w, from, &moves, &[(2, shift())]);
    let path = positions(&w, 0);
    assert_eq!(path.len(), 9, "the press and every move: {path:?}");
    let start = path[0];
    assert!(
        (path[3..].iter()).all(|p| ((p[1] - start[1]) - (p[0] - start[0])).abs() < 1e-3),
        "from Shift on, on the 45 degree line from the press: {path:?}"
    );
    assert!(
        ((path[1][1] - start[1]) - (path[1][0] - start[0])).abs() > 1.0,
        "before Shift, where the pen was: {path:?}"
    );
}

/// A slot's stroke holds a line as the session brush's does: Shift after a Ctrl press holds the
/// Mask Pen's line; a Smooth stroke, whose slot Shift calls, holds one only once Shift is let go
/// and pressed again.
#[test]
fn a_slot_stroke_holds_a_line_too() {
    let (mut w, rect) = with_sphere(Some(log_path("slot-line")));
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    let moves: Vec<[f32; 2]> = (1..=10_u8)
        .map(|k| {
            let k = f32::from(k);
            [cx - 60.0 + 10.0 * k, cy + 8.0 * (k - 5.0).max(0.0)]
        })
        .collect();
    w.input(&[], ctrl(false));
    stroke(&mut w, [cx - 60.0, cy], &moves, &[(4, ctrl(true))]);
    let line = positions(&w, 0);
    assert_eq!(brush(&w, 0), json!("Mask Pen"));
    assert!(
        line.len() == 11 && line.iter().all(|p| (p[1] - line[0][1]).abs() < 1e-3),
        "the Mask Pen's samples on its first line: {line:?}"
    );
    // Smooth, Shift held from the press: it follows the pen, Shift let go before the third move
    // and pressed again before the fifth holds it to the way it first went from there.
    let p0 = [cx - 40.0, cy - 30.0];
    let steps = [
        [10.0, 6.0],
        [20.0, 0.0],
        [30.0, 6.0],
        [40.0, 12.0],
        [50.0, 18.0],
        [60.0, 18.0],
        [70.0, 18.0],
        [80.0, 18.0],
    ];
    let moves: Vec<[f32; 2]> = (steps.iter())
        .map(|[x, y]| [p0[0] + x, p0[1] + y])
        .collect();
    w.input(&[], shift());
    let none = Modifiers::default();
    stroke(&mut w, p0, &moves, &[(2, none), (4, shift())]);
    let path = positions(&w, 1);
    assert_eq!(brush(&w, 1), json!("Smooth"));
    assert_eq!(path.len(), 9, "the press and every move: {path:?}");
    assert!(
        (path[2][1] - path[0][1]).abs() < 1e-3,
        "Shift from the press holds no line: {path:?}"
    );
    let (from, way) = (path[5], [10.0 / 136_f64.sqrt(), 6.0 / 136_f64.sqrt()]);
    let off = |p: &[f64; 2]| ((p[0] - from[0]) * way[1] - (p[1] - from[1]) * way[0]).abs();
    assert!(
        path[5..].iter().all(|p| off(p) < 1e-3) && path[8][1] > path[5][1] + 5.0,
        "pressed again, on the line the stroke first took: {path:?}"
    );
}
