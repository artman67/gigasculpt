//! The curve line's points in the window (decision 0055): a bare Alt tap during a Clip Curve
//! stroke adds a point to the stroke and leaves the next drag on empty canvas orbiting, not
//! dollying; a quick second tap makes the point a corner; Space held during the stroke moves
//! the path and never opens the palette; Shift then holds the line from the latest point.

use gs_engine::ExactCamera;
use gs_platform::{Input, Modifiers};
use gs_render::Camera;
use serde_json::{Value, json};

use crate::region;
use crate::rig::{Window, log_path, pen_at, with_sphere};

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Modifiers::default()
    }
}

/// Space going down or up.
pub(crate) fn space(down: bool) -> Input {
    Input::Key {
        key: "Space".into(),
        down,
        repeat: false,
        modifiers: Modifiers::default(),
    }
}

/// Alt pressed and let go, a frame each.
fn tap(w: &mut Window) {
    w.input(&[], alt());
    w.input(&[], Modifiers::default());
}

/// A Clip Curve stroke across the 3D view's middle, the window drawing a frame after each
/// sample: a hover, then ten samples touching (logged as samples 0 to 9), `before(w, k)`
/// running before the `k`th (1 to 10), then the lift.
fn stroke(w: &mut Window, rect: [f32; 4], mut before: impl FnMut(&mut Window, usize)) {
    let [cx, cy] = region::middle(rect.map(f64::from)).map(|v| v as f32);
    let at = |k: usize| {
        let y = if k.is_multiple_of(2) { cy } else { cy + 10.0 };
        pen_at(4 * k as u64, cx - 80.0 + 16.0 * k as f32, y, 0.5)
    };
    let mut hover = at(0);
    (hover.contact, hover.pressure) = (false, 0.0);
    w.pen(hover);
    w.input(&[], Modifiers::default());
    for k in 1..=10 {
        before(w, k);
        w.pen(at(k));
        w.input(&[], Modifiers::default());
    }
    let mut up = at(10);
    (up.contact, up.pressure, up.t_ns) = (false, 0.0, up.t_ns + 4_000_000);
    w.pen(up);
    w.input(&[], Modifiers::default());
}

/// The samples of the last logged brush stroke.
fn samples(w: &Window) -> Vec<Value> {
    let logged = w.logged();
    let last = logged.iter().rev().find(|l| l.0 == "brush.stroke");
    (last.and_then(|l| l.1["samples"].as_array().cloned())).unwrap_or_default()
}

/// Each sample of the last logged stroke that carries a point, and its kind.
fn points(w: &Window) -> Vec<(usize, String)> {
    (samples(w).iter().enumerate())
        .filter_map(|(i, s)| s["point"].as_str().map(|p| (i, p.to_string())))
        .collect()
}

fn camera(w: &mut Window) -> Camera {
    let v = w.ok("view.get", json!({ "view": "main" }))["camera"].clone();
    let exact: ExactCamera = serde_json::from_value(v).unwrap_or_else(|e| panic!("{e}"));
    exact.camera("").unwrap_or_else(|e| panic!("{}", e.message))
}

/// A drag up with the pen's tip from near the view's bottom left corner (empty canvas), a frame
/// after each sample.
fn drag_on_canvas(w: &mut Window, rect: [f32; 4]) {
    let (x, y) = (rect[0] + 30.0, rect[3] - 30.0);
    for k in 0..6_u8 {
        let pressure = if k == 0 || k == 5 { 0.0 } else { 0.5 };
        let at = pen_at(
            1000 + 4 * u64::from(k),
            x,
            y - 12.0 * f32::from(k),
            pressure,
        );
        w.pen(at);
        w.input(&[], Modifiers::default());
    }
}

#[test]
fn a_bare_alt_tap_mid_curve_stroke_adds_a_point_and_the_next_drag_still_orbits() {
    let (mut w, rect) = with_sphere(Some(log_path("curve-tap")));
    w.ok("brush.set", json!({ "brush": "Clip Curve" }));
    stroke(&mut w, rect, |w, k| {
        if k == 4 {
            tap(w);
        }
    });
    assert_eq!(
        points(&w),
        [(2, "soft".to_string())],
        "the tap's point sits on the sample before it"
    );
    // A gesture pressed on empty canvas draws there (decision 0017 as amended 2026-10-12, item
    // 17), so the drag that turns the view is a surface brush's.
    w.ok("brush.set", json!({ "brush": "Draw" }));
    let before = camera(&mut w);
    drag_on_canvas(&mut w, rect);
    let after = camera(&mut w);
    assert_ne!(after.orientation, before.orientation, "the drag orbits");
    assert_eq!(after.distance, before.distance, "and does not dolly");
}

#[test]
fn a_quick_second_alt_tap_makes_the_point_a_corner() {
    let (mut w, rect) = with_sphere(Some(log_path("curve-corner")));
    w.ok("brush.set", json!({ "brush": "Clip Curve" }));
    stroke(&mut w, rect, |w, k| match k {
        4 => {
            tap(w);
            tap(w);
        }
        7 => {
            w.wait(0.5);
            tap(w);
            w.wait(0.5);
            tap(w);
        }
        _ => {}
    });
    let want = [(2, "sharp".to_string()), (5, "soft".to_string())];
    assert_eq!(points(&w), want, "taps far apart add no corner");
}

#[test]
fn space_mid_curve_stroke_moves_the_path_and_never_opens_the_palette() {
    let (mut w, rect) = with_sphere(Some(log_path("curve-space")));
    w.ok("brush.set", json!({ "brush": "Clip Curve" }));
    let mut opened = false;
    stroke(&mut w, rect, |w, k| {
        match k {
            5 => w.input(&[space(true)], Modifiers::default()),
            8 => w.input(&[space(false)], Modifiers::default()),
            _ => {}
        }
        opened |= w.host.lock().shell.palette.open;
    });
    assert!(!opened, "Space during the stroke left the palette shut");
    let moving: Vec<usize> = (samples(&w).iter().enumerate())
        .filter(|(_, s)| s["moves"] == json!(true))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        moving,
        [4, 5, 6],
        "the samples drawn with Space held move the path"
    );
}

/// Shift after an Alt tap holds the curve to the 45 degree line from that point, not from the
/// press (decision 0055, section 3).
#[test]
fn shift_after_a_tap_holds_the_line_from_the_point() {
    let (mut w, rect) = with_sphere(Some(log_path("curve-shift")));
    w.ok("brush.set", json!({ "brush": "Clip Curve" }));
    let [cx, cy] = region::middle(rect.map(f64::from)).map(|v| v as f32);
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    // Down the 45 degree diagonal, a tap, then Shift held going right and a little down: about
    // 10 degrees from the point (so level), about 30 from the press (so the diagonal).
    let moves = [
        (0.0, 0.0),
        (30.0, 30.0),
        (60.0, 60.0),
        (90.0, 65.0),
        (120.0, 70.0),
    ];
    w.pen(pen_at(0, cx - 60.0, cy - 40.0, 0.0));
    w.input(&[], Modifiers::default());
    for (k, (x, y)) in moves.into_iter().enumerate() {
        if k == 3 {
            tap(&mut w);
            w.input(&[], shift);
        }
        let at = pen_at(4 + 4 * k as u64, cx - 60.0 + x, cy - 40.0 + y, 0.5);
        w.pen(at);
        w.input(&[], if k >= 3 { shift } else { Modifiers::default() });
    }
    w.pen(pen_at(40, cx + 60.0, cy + 30.0, 0.0));
    w.input(&[], Modifiers::default());
    assert_eq!(points(&w), [(2, "soft".to_string())], "the tap's point");
    let y = |s: &Value| s["position"][1].as_f64().unwrap_or_default();
    let got: Vec<f64> = samples(&w).iter().map(y).collect();
    assert!(
        got.len() == 5 && (got[3] - got[2]).abs() < 1e-3 && (got[4] - got[2]).abs() < 1e-3,
        "from Shift on, level with the point: {got:?}"
    );
}
