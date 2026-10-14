//! Navigating the 3D view with the mouse, through the window's gesture map and the engine's
//! View commands (decision 0017): the right button orbits by the speed setting (0.4584 degrees,
//! 0.008 radians, per point), with Alt it pans so the pivot follows the pointer and with Ctrl it
//! dollies 0.5% per point; the middle button pans; the wheel dollies 10% per notch; there is no
//! stop at straight up or down; and the pen's cursor ring sits at the pen.

use gs_core::Vec3;
use gs_engine::Engine;
use gs_platform::{Button, Input, Modifiers as Held};
use gs_protocol::PenSample;
use gs_render::{BACKGROUND, Camera};
use gs_testkit::compare::within;
use gs_ui::Host;
use serde_json::json;

use crate::canvas::{at, camera, drag, middle, pump, rect, run, shot, studio, view};
use crate::region::{self, region};

/// A studio with a sphere, and its camera.
fn navigable() -> (Engine, Host, Camera) {
    let (mut engine, host) = studio(1.0);
    run(&mut engine, false, "primitive.sphere", json!({}));
    pump(&mut engine, &host);
    let c = camera(&mut engine);
    (engine, host, c)
}

/// The 3D view's middle, `m`, moved by `d` points.
fn by(m: [f64; 2], d: [f64; 2]) -> [f64; 2] {
    [m[0] + d[0], m[1] + d[1]]
}

/// `a` within a thousandth of `b` (relative, or absolute below 1).
fn close(a: f64, b: f64, what: &str) {
    let scale = b.abs().max(1.0);
    assert!(within((a - b).abs() / scale, 1e-3), "{what}: {a} vs {b}");
}

/// The angle between two unit vectors, in radians.
fn angle(a: Vec3, b: Vec3) -> f64 {
    libm::acos(f64::from(a.dot(b).clamp(-1.0, 1.0)))
}

/// Pixels per point of drag at the target's depth, in scene units.
fn per_point(c: &Camera, height: f64) -> f64 {
    let tan = f64::from(c.lens.tan_half(1.0)[1]);
    2.0 * f64::from(c.distance) * tan / height
}

#[test]
fn a_right_drag_orbits_by_the_speed_setting_and_turns_about_the_screen() {
    let (mut engine, host, c0) = navigable();
    let m = middle(&mut engine).map(f64::from);
    let (_, up0, f0) = c0.axes();
    drag(
        &mut engine,
        &host,
        Button::Right,
        Held::default(),
        by(m, [0.0, 0.0]),
        by(m, [100.0, 0.0]),
    );
    let c1 = camera(&mut engine);
    let (_, up1, f1) = c1.axes();
    close(
        angle(f0, f1),
        0.8,
        "100 points across turn the view 0.8 rad",
    );
    close(angle(up0, up1), 0.0, "about the screen's up axis");
    assert!(
        f1.x > 0.0,
        "the camera moved to its left, looking more toward +X: {f1:?}"
    );
    drag(
        &mut engine,
        &host,
        Button::Right,
        Held::default(),
        by(m, [0.0, 0.0]),
        by(m, [0.0, 50.0]),
    );
    let (_, _, f2) = camera(&mut engine).axes();
    close(angle(f1, f2), 0.4, "50 points down turn it 0.4 rad");
    assert!(f2.y < f1.y, "and it looks more from above: {f2:?}");
}

#[test]
fn alt_drag_pans_with_the_pointer_ctrl_drag_dollies_and_middle_drag_pans() {
    let (mut engine, host, c0) = navigable();
    let m = middle(&mut engine).map(f64::from);
    let [_, y0, _, y1] = rect(&view(&mut engine));
    let alt = Held {
        alt: true,
        ..Held::default()
    };
    let ctrl = Held {
        ctrl: true,
        ..Held::default()
    };
    drag(
        &mut engine,
        &host,
        Button::Right,
        alt,
        by(m, [0.0, 0.0]),
        by(m, [60.0, 0.0]),
    );
    let c1 = camera(&mut engine);
    let moved = Vec3::from_array(c1.target) - Vec3::from_array(c0.target);
    let (right, _, _) = c0.axes();
    close(
        f64::from(moved.dot(right)),
        -60.0 * per_point(&c0, y1 - y0),
        "60 points right",
    );
    close(angle(c0.axes().2, c1.axes().2), 0.0, "no turn");
    drag(
        &mut engine,
        &host,
        Button::Middle,
        Held::default(),
        by(m, [0.0, 0.0]),
        by(m, [0.0, 60.0]),
    );
    let c2 = camera(&mut engine);
    let moved = Vec3::from_array(c2.target) - Vec3::from_array(c1.target);
    close(
        f64::from(moved.dot(c1.axes().1)),
        60.0 * per_point(&c1, y1 - y0),
        "60 points down",
    );
    drag(
        &mut engine,
        &host,
        Button::Right,
        ctrl,
        by(m, [0.0, 0.0]),
        by(m, [0.0, 100.0]),
    );
    let c3 = camera(&mut engine);
    close(
        f64::from(c3.distance / c2.distance),
        libm::pow(1.005, 100.0),
        "100 points down back away 0.5% each",
    );
}

#[test]
fn the_wheel_dollies_over_the_view_and_nothing_stops_at_the_poles() {
    let (mut engine, host, c0) = navigable();
    let m = middle(&mut engine).map(f64::from);
    let top = rect(&view(&mut engine))[1];
    let wheel = |at: [f64; 2], dy| {
        [
            Input::MouseMove { x: at[0], y: at[1] },
            Input::Wheel { dx: 0.0, dy },
        ]
    };
    host.lock().input(&wheel(m, 1.0), Held::default());
    pump(&mut engine, &host);
    let c1 = camera(&mut engine);
    close(
        f64::from(c1.distance / c0.distance),
        0.9,
        "one notch: 10% closer",
    );
    let panel = region::middle(region(
        &run(&mut engine, false, "ui.describe", json!({})),
        "brush",
    ));
    host.lock().input(&wheel(panel, 3.0), Held::default());
    pump(&mut engine, &host);
    close(
        f64::from(camera(&mut engine).distance),
        f64::from(c1.distance),
        "the wheel over a panel",
    );
    // Down 200 points twice: 0.008 * 400 = 3.2 rad, past straight down and over.
    for _ in 0..2 {
        drag(
            &mut engine,
            &host,
            Button::Right,
            Held::default(),
            [m[0], top + 16.0],
            [m[0], top + 216.0],
        );
    }
    let (_, up, _) = camera(&mut engine).axes();
    assert!(
        up.y < 0.0,
        "the view turned over the pole, upside down: {up:?}"
    );
}

#[test]
fn shift_during_an_orbit_lands_on_an_axis_view() {
    let (mut engine, host, _) = navigable();
    let m = middle(&mut engine).map(f64::from);
    let shift = Held {
        shift: true,
        ..Held::default()
    };
    drag(
        &mut engine,
        &host,
        Button::Right,
        shift,
        by(m, [0.0, 0.0]),
        by(m, [70.0, -20.0]),
    );
    let (_, up, forward) = camera(&mut engine).axes();
    let axis = |v: Vec3| [v.x, v.y, v.z].iter().any(|c| (c.abs() - 1.0).abs() < 1e-5);
    assert!(
        axis(forward) && axis(up),
        "an axis view: {forward:?}, {up:?}"
    );
}

#[test]
fn the_cursor_ring_sits_at_the_pen_and_its_disc_grows_with_pressure() {
    // The pen hovers, then touches, in the middle of the empty 3D view.
    let (mut engine, _host) = studio(2.0);
    let [x, y] = middle(&mut engine);
    let hover = json!({ "samples": [{ "x": x, "y": y }] });
    run(&mut engine, false, "pen.input", hover);
    let image = shot(&mut engine);
    let ring = at(&image, x + 20, y);
    assert!(
        ring[2] > ring[0] + 40,
        "10 points (20 pixels) right of the pen: {ring:?}"
    );
    let (mut engine, _host) = studio(1.0);
    let [x, y] = middle(&mut engine);
    let touch = json!({ "samples": [{ "x": x, "y": y, "contact": true, "pressure": 0.5 }] });
    run(&mut engine, false, "pen.input", touch);
    let image = shot(&mut engine);
    let inside = at(&image, x + 4, y);
    assert!(
        inside[0] > 120,
        "half pressure: a 5-point disc covers 4 points out: {inside:?}"
    );
    assert_eq!(at(&image, x + 7, y), BACKGROUND, "but not 7 points out");
}

/// A pen sample at (`x`, `y`) with the barrel button held or not.
fn barrel(x: f64, y: f64, held: bool) -> PenSample {
    PenSample {
        buttons: u8::from(held),
        ..PenSample::hover(0, x, y)
    }
}

#[test]
fn the_pen_barrel_orbits_over_the_model_and_its_click_opens_the_brush_wheel() {
    let (mut engine, host, c0) = navigable();
    let m = middle(&mut engine).map(f64::from);
    for s in [
        barrel(m[0], m[1], false),
        barrel(m[0], m[1], true),
        barrel(m[0] + 50.0, m[1], true),
        barrel(m[0] + 100.0, m[1], true),
        barrel(m[0] + 100.0, m[1], false),
    ] {
        run(&mut engine, true, "pen.input", json!({ "samples": [s] }));
        pump(&mut engine, &host);
    }
    let c1 = camera(&mut engine);
    close(
        angle(c0.axes().2, c1.axes().2),
        0.8,
        "100 points turn the view 0.8 rad",
    );
    for s in [
        barrel(m[0] - 100.0, m[1] - 50.0, true),
        barrel(m[0] - 100.0, m[1] - 50.0, false),
    ] {
        run(&mut engine, true, "pen.input", json!({ "samples": [s] }));
        pump(&mut engine, &host);
    }
    assert!(
        host.lock().shell.wheel.at.is_some(),
        "a click opens the wheel (the default)"
    );
    run(
        &mut engine,
        true,
        "ui.navigation",
        json!({ "barrel_click": "palette" }),
    );
    for s in [
        barrel(m[0] - 100.0, m[1] - 50.0, true),
        barrel(m[0] - 100.0, m[1] - 50.0, false),
    ] {
        run(&mut engine, true, "pen.input", json!({ "samples": [s] }));
        pump(&mut engine, &host);
    }
    assert!(
        host.lock().shell.palette.open,
        "switched: a click opens the palette"
    );
    run(&mut engine, true, "ui.palette", json!({ "open": false }));
    let quick = json!({ "barrel_click": "quick" });
    run(&mut engine, true, "ui.navigation", quick);
    for s in [
        barrel(m[0] - 100.0, m[1] - 50.0, true),
        barrel(m[0] - 100.0, m[1] - 50.0, false),
    ] {
        run(&mut engine, true, "pen.input", json!({ "samples": [s] }));
        pump(&mut engine, &host);
    }
    assert!(
        host.lock().shell.quick.is_some(),
        "switched: a click opens the quick menu"
    );
}
