//! Sculpting with the pen in the window: a touch on the 3D view sculpts while the pen moves,
//! lifting it makes one undo step and one `brush.stroke` log line, and that line run as a
//! command gives the same mesh bit for bit. Agent edits wait while the pen is down. Held
//! modifiers pick what the stroke does. The mouse's left button is the tip: it sculpts on the
//! model and turns the view on empty canvas.

use gs_platform::{Button, Input, Modifiers};
use gs_protocol::{ErrorCode, Outcome, Role};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::rig::{Window, across, log_path, pen_at, with_sphere};
use gs_ui::{MousePen, split_pen};

/// A windowed session with a sphere drawn in the view.
#[test]
fn a_pen_stroke_is_the_brush_stroke_it_logs() {
    let (mut w, rect) = with_sphere(Some(log_path("pen")));
    let before = w.hash();
    for s in across(rect, 12) {
        w.pen(s);
    }
    let after = w.hash();
    assert_ne!(after, before, "the stroke sculpted the sphere");
    let logged = w.logged();
    let commands: Vec<&str> = logged.iter().map(|l| l.0.as_str()).collect();
    // The framing the window's view keeps is logged once (its size is not in the log).
    assert_eq!(
        commands,
        ["primitive.sphere", "view.set", "pen.input", "brush.stroke"],
        "the hover is logged as pen input, the touching samples only in their stroke"
    );
    let stroke = logged[3].1.clone();
    assert_eq!(
        stroke["samples"].as_array().map(Vec::len),
        Some(13),
        "{stroke}"
    );
    assert_eq!(stroke["brush"], json!("Draw"), "the session's brush");
    // The same call, run as a command in a session without a window.
    let mut headless = Session::new();
    headless.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    headless.ok("brush.stroke", stroke);
    assert_eq!(
        headless.hash(),
        after,
        "the logged call gives the same mesh"
    );
    let steps = w.ok("history.list", json!({}))["steps"].clone();
    assert_eq!(
        steps.as_array().map(Vec::len),
        Some(2),
        "the sphere, then the stroke"
    );
    w.ok("history.undo", json!({}));
    assert_eq!(w.hash(), before, "one undo takes back the whole stroke");
}

/// The window draws a pen stroke, fed to its GPU copy frame by frame as the pen moves, exactly
/// as it draws the same stroke run as one command.
#[test]
fn the_window_draws_a_pen_stroke_as_it_draws_the_logged_call() {
    let (mut w, rect) = with_sphere(Some(log_path("pen-drawn")));
    let before = w.view_pixels();
    let path = across(rect, 12);
    for &s in &path {
        w.pen(s);
    }
    if let Some(&last) = path.last() {
        // Out of range, so no cursor is drawn over the model.
        w.pen(gs_protocol::PenSample {
            in_proximity: false,
            ..last
        });
    }
    let sculpted = w.view_pixels();
    assert!(sculpted != before, "the stroke shows");
    let logged = w.logged().into_iter().find(|l| l.0 == "brush.stroke");
    let stroke = logged.map_or_else(|| panic!("the stroke is logged"), |l| l.1);
    let (mut replay, _) = with_sphere(None);
    // Drawn before the stroke too, so the camera is framed on the same sphere.
    assert!(
        replay.view_pixels() == before,
        "the same sphere, the same view"
    );
    replay.ok("brush.stroke", stroke);
    assert!(
        replay.view_pixels() == sculpted,
        "the pen stroke and its logged call draw the same pixels"
    );
}

#[test]
fn the_part_changes_under_the_pen_and_agents_wait() {
    let (mut w, rect) = with_sphere(None);
    let before = w.hash();
    let path = across(rect, 12);
    for &s in &path[..6] {
        w.pen(s);
    }
    assert_ne!(w.hash(), before, "the part changed before the pen lifted");
    let steps = |w: &mut Window| w.ok("history.list", json!({}))["steps"].clone();
    assert_eq!(
        steps(&mut w).as_array().map(Vec::len),
        Some(1),
        "no step yet"
    );
    let edit = json!({ "direction": [1, 0, 0], "distance": 0.1 });
    let refused = w.call_as(Role::Agent, "mesh.translate", edit.clone());
    let Outcome::Error(e) = refused.outcome else {
        panic!("an agent edit while the pen is down must wait");
    };
    assert_eq!(e.code, ErrorCode::Busy, "{}", e.message);
    let looked = w.call_as(Role::Agent, "mesh.stats", json!({}));
    assert!(
        matches!(looked.outcome, Outcome::Ok(_)),
        "queries still work"
    );
    for &s in &path[6..] {
        w.pen(s);
    }
    assert_eq!(
        steps(&mut w).as_array().map(Vec::len),
        Some(2),
        "one step for the stroke"
    );
    let done = w.call_as(Role::Agent, "mesh.translate", edit);
    assert!(
        matches!(done.outcome, Outcome::Ok(_)),
        "the agent goes on after the lift"
    );
}

/// The arguments of the stroke drawn across the view with `held` modifiers.
fn stroke_with(held: Modifiers) -> serde_json::Value {
    let (mut w, rect) = with_sphere(Some(log_path("held")));
    w.input(&[], held);
    for s in across(rect, 6) {
        w.pen(s);
    }
    let logged = w.logged();
    logged.last().map(|l| l.1.clone()).unwrap_or_default()
}

/// With Live Detail on, the pen refines as it sculpts: its logged call carries every setting,
/// the window draws the stroke as it draws that call run alone, and undo shows the sphere as
/// it was.
#[test]
fn with_live_detail_on_the_pen_refines_and_logs_every_setting() {
    let (mut w, rect) = with_sphere(Some(log_path("pen-detail")));
    w.ok(
        "live_detail.set",
        json!({ "on": true, "mode": "screen", "size": 8 }),
    );
    let before = w.view_pixels();
    let path = across(rect, 12);
    for &s in &path {
        w.pen(s);
    }
    if let Some(&last) = path.last() {
        // Out of range, so no cursor is drawn over the model.
        w.pen(gs_protocol::PenSample {
            in_proximity: false,
            ..last
        });
    }
    let sculpted = w.view_pixels();
    let logged = w.logged().into_iter().find(|l| l.0 == "brush.stroke");
    let stroke = logged.map_or_else(|| panic!("the stroke is logged"), |l| l.1);
    let settings = json!({ "mode": "screen", "size": 8.0, "merge": 0.4_f32, "combined": true,
                           "budget": 4096, "smooth_split": true, "relax": 0.5 });
    assert_eq!(stroke["live_detail"], settings, "{stroke}");
    let (mut replay, _) = with_sphere(None);
    assert!(
        replay.view_pixels() == before,
        "the same sphere, the same view"
    );
    let done = replay.ok("brush.stroke", stroke);
    let splits = done["live_detail"]["splits"].as_u64().unwrap_or_default();
    assert!(splits > 0, "the stroke refined: {done}");
    assert_eq!(
        replay.hash(),
        w.hash(),
        "the logged call gives the same mesh"
    );
    // Compared with assert!, not assert_eq!: a failure would print millions of bytes.
    assert!(
        replay.view_pixels() == sculpted,
        "the pen stroke and its logged call draw the same pixels"
    );
    w.ok("history.undo", json!({}));
    assert!(w.view_pixels() == before, "undo shows the sphere as it was");
}

#[test]
fn held_modifiers_pick_what_the_stroke_does() {
    let plain = stroke_with(Modifiers::default());
    assert_eq!(
        (plain["brush"].clone(), plain.get("invert")),
        (json!("Draw"), None)
    );
    let alt = stroke_with(Modifiers {
        alt: true,
        ..Modifiers::default()
    });
    assert_eq!(
        (alt["brush"].clone(), alt["invert"].clone()),
        (json!("Draw"), json!(true))
    );
    let shift = stroke_with(Modifiers {
        shift: true,
        ..Modifiers::default()
    });
    assert_eq!(shift["brush"], json!("Smooth"), "Shift smooths");
    let ctrl = stroke_with(Modifiers {
        ctrl: true,
        ..Modifiers::default()
    });
    assert_eq!(ctrl["brush"], json!("Mask Pen"), "Ctrl masks");
    let both = stroke_with(Modifiers {
        shift: true,
        ctrl: true,
        ..Modifiers::default()
    });
    assert_eq!(both["brush"], json!("Select Rect"), "Shift+Ctrl selects");
}

/// A brush put on a slot strokes while the slot's key is held, and the window says so first
/// (`brush-settings.modifier-key-slots`).
#[test]
fn a_slot_strokes_with_the_brush_put_on_it() {
    let (mut w, rect) = with_sphere(Some(log_path("slot")));
    w.ok(
        "brush.slot",
        json!({ "slot": "smooth", "brush": "Flat Polish" }),
    );
    w.input(
        &[],
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    for s in across(rect, 6) {
        w.pen(s);
    }
    let logged = w.logged();
    let hold = json!({ "slot": "smooth", "hold": true });
    assert!(
        logged.iter().any(|l| l.0 == "brush.slot" && l.1 == hold),
        "{logged:?}"
    );
    let stroke = logged.iter().rev().find(|l| l.0 == "brush.stroke");
    assert_eq!(
        stroke.map(|l| l.1["brush"].clone()),
        Some(json!("Flat Polish"))
    );
}

/// Barrel button 2 set to stand for Shift (`ui.navigation` `barrel_two`) smooths exactly as the
/// key does (`interface.modifier-keys`): the same stroke made with it held, and made with Shift
/// held, leaves the same mesh. Set to pan, as it starts, it pans and sculpts nothing.
#[test]
fn a_pen_button_bound_to_shift_smooths_exactly_as_the_key_does() {
    let stroke = |button: bool| {
        let (mut w, rect) = with_sphere(Some(log_path("barrel-shift")));
        let shift = Modifiers {
            shift: !button,
            ..Modifiers::default()
        };
        if button {
            w.ok("ui.navigation", json!({ "barrel_two": "shift" }));
        }
        w.input(&[], shift);
        let before = w.hash();
        for s in across(rect, 6) {
            w.pen(gs_protocol::PenSample {
                buttons: if button { 2 } else { 0 },
                ..s
            });
            w.input(&[], shift);
        }
        let logged = w.logged();
        let stroke = logged.iter().rev().find(|l| l.0 == "brush.stroke");
        let brush = stroke.map(|l| l.1["brush"].clone());
        (before, w.hash(), brush)
    };
    let (before, key, brush) = stroke(false);
    assert_ne!(key, before, "Shift held smooths");
    assert_eq!(brush, Some(json!("Smooth")));
    let (_, button, brush) = stroke(true);
    assert_eq!(button, key, "the button smooths as the key does");
    assert_eq!(brush, Some(json!("Smooth")));
    // As it starts, barrel button 2 pans: no stroke.
    let (mut w, rect) = with_sphere(None);
    let before = w.hash();
    for s in across(rect, 6) {
        w.pen(gs_protocol::PenSample { buttons: 2, ..s });
        w.input(&[], Modifiers::default());
    }
    assert_eq!(w.hash(), before, "it pans");
}

/// Alt pressed partway through a pen stroke, then released, works the other way on the
/// samples between only (decision 0036 section 5): the logged stroke carries `invert` on those
/// samples, and run alone it gives the same mesh.
#[test]
fn alt_pressed_mid_stroke_changes_only_the_samples_after_it_and_replays() {
    let (mut w, rect) = with_sphere(Some(log_path("alt-mid")));
    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    let path = across(rect, 12);
    // The hover and seven touching samples, three with Alt held, and the rest without.
    for (k, &s) in path.iter().enumerate() {
        match k {
            8 => w.input(&[], alt),
            11 => w.input(&[], Modifiers::default()),
            _ => {}
        }
        w.pen(s);
    }
    let logged = w.logged().into_iter().find(|l| l.0 == "brush.stroke");
    let stroke = logged.map_or_else(|| panic!("the stroke is logged"), |l| l.1);
    assert_eq!(stroke.get("invert"), None, "the stroke started without Alt");
    let signs: Vec<Option<bool>> = (stroke["samples"].as_array().into_iter().flatten())
        .map(|s| s["invert"].as_bool())
        .collect();
    let want = [vec![None; 7], vec![Some(true); 3], vec![None; 3]].concat();
    assert_eq!(signs, want, "only the samples while Alt was held carry it");
    let mut headless = Session::new();
    headless.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    headless.ok("brush.stroke", stroke);
    assert_eq!(
        headless.hash(),
        w.hash(),
        "the logged call gives the same mesh"
    );
}

#[test]
fn the_pen_sculpts_only_the_view() {
    let (mut w, _) = with_sphere(Some(log_path("outside")));
    let before = w.hash();
    // On the status line, below the view.
    let [x, y] = crate::region::middle(w.rect("status")).map(|v| v as f32);
    for (t, dx) in [(0, 0.0), (4, 20.0), (8, 40.0)] {
        w.pen(pen_at(t, x + dx, y, 0.8));
    }
    w.pen(pen_at(12, x + 40.0, y, 0.0));
    assert_eq!(w.hash(), before, "nothing sculpted");
    assert!(
        w.logged().iter().all(|l| l.0 != "brush.stroke"),
        "no stroke"
    );
}

/// The window pixel on the middle row where `part` first shows, scanning from the left.
fn first_pixel_of(w: &mut Window, rect: [f32; 4], part: &str) -> [f32; 2] {
    let y = f32::midpoint(rect[1], rect[3]);
    let found = (0..64)
        .map(|k| rect[0] + (rect[2] - rect[0]) * k as f32 / 64.0)
        .find(|&x| w.picked(x, y)["hit"]["part"] == json!(part));
    [
        found.unwrap_or_else(|| panic!("{part} is not on the middle row")),
        y,
    ]
}

#[test]
fn with_several_parts_the_stroke_takes_the_active_one() {
    let mut w = Window::new(Some(log_path("parts")));
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    w.ok(
        "primitive.cube",
        json!({ "center": [2.5, 0.0, 0.0], "subdivisions": 8 }),
    );
    let rect = w.view_rect();
    let sphere = w.ok("part.get", json!({ "part": "sphere" }));
    // The cube, added last, is the active part (decision 0033), the only one a stroke takes.
    let [x, y] = first_pixel_of(&mut w, rect, "cube");
    let path = [
        (0, x + 6.0, 0.0),
        (4, x + 6.0, 0.8),
        (8, x + 16.0, 0.8),
        (12, x + 16.0, 0.0),
    ];
    for (t, px, p) in path {
        w.pen(pen_at(t, px, y, p));
    }
    let logged = w.logged();
    let stroke = logged.last().map(|l| l.1.clone()).unwrap_or_default();
    assert_eq!(
        stroke["part"],
        json!("cube"),
        "the part under the pen: {stroke}"
    );
    assert_eq!(
        w.ok("part.get", json!({ "part": "sphere" })),
        sphere,
        "the sphere is untouched"
    );
    // Off both parts: nothing to stroke, nothing logged.
    let before = w.hash();
    let off = [
        (20, rect[0] + 3.0, 0.0),
        (24, rect[0] + 3.0, 0.8),
        (28, rect[0] + 9.0, 0.0),
    ];
    for (t, px, p) in off {
        w.pen(pen_at(t, px, rect[1] + 3.0, p));
    }
    assert_eq!(
        w.hash(),
        before,
        "a stroke that touches no part changes nothing"
    );
    assert_eq!(
        w.logged().len(),
        logged.len() + 1,
        "only the hover is logged: {:?}",
        w.logged()
    );
}

#[test]
fn the_logged_stroke_keeps_the_modifier_falloff_tilt_and_order() {
    let (mut w, rect) = with_sphere(Some(log_path("tilt")));
    w.ok("brush.set", json!({ "modifier": 40, "falloff": 30 }));
    // The hover and the first touch upright, then tilted one way and the other.
    let mut path = across(rect, 4);
    for (k, s) in path.iter_mut().enumerate().skip(2) {
        (s.tilt_x, s.tilt_y) = if k % 2 == 0 {
            (12.5, -30.0)
        } else {
            (8.0, 0.0)
        };
    }
    for s in path {
        w.pen(s);
    }
    // The stroke carries the brush's patch: what the session changed, in the preset's names.
    let stroke = w.logged().last().map(|l| l.1.clone()).unwrap_or_default();
    assert_eq!(stroke["settings"]["modifier"], json!(40.0), "{stroke}");
    assert_eq!(
        stroke["settings"]["tip"]["focal"],
        json!(30.0),
        "Draw's Falloff, as set: {stroke}"
    );
    let tilts: Vec<Option<&Value>> = (0..3).map(|i| stroke["samples"][i].get("tilt")).collect();
    assert_eq!(
        tilts,
        [None, Some(&json!([12.5, -30.0])), Some(&json!([8.0, 0.0]))],
        "an upright pen logs no tilt: {stroke}"
    );
    // Smooth takes no Brush Modifier: the stroke leaves it out rather than fail.
    let held = gs_platform::Modifiers {
        shift: true,
        ..gs_platform::Modifiers::default()
    };
    w.input(&[], held);
    for s in across(rect, 4) {
        w.pen(s);
    }
    let smooth = w.logged().last().map(|l| l.1.clone()).unwrap_or_default();
    assert_eq!(
        (smooth["brush"].clone(), smooth.get("modifier")),
        (json!("Smooth"), None)
    );
    assert_eq!(
        smooth.get("falloff"),
        None,
        "Smooth's own Falloff, never set: {smooth}"
    );
    // Every call has a number of its own, rising: a stroke's included.
    let seqs: Vec<u64> = w.records().iter().filter_map(|r| r.seq).collect();
    assert!(
        seqs.windows(2).all(|p| p[0] < p[1]),
        "sequence numbers rise: {seqs:?}"
    );
}

#[test]
fn an_edit_or_an_undo_while_the_pen_is_down_ends_the_stroke_first() {
    let (mut w, rect) = with_sphere(Some(log_path("midway")));
    let before = w.hash();
    let path = across(rect, 8);
    for &s in &path[..5] {
        w.pen(s);
    }
    w.ok("history.undo", json!({}));
    assert_eq!(w.hash(), before, "the undo took back the stroke so far");
    for &s in &path[5..] {
        w.pen(s);
    }
    let commands: Vec<String> = w.logged().into_iter().map(|l| l.0).collect();
    assert_eq!(
        commands,
        [
            "primitive.sphere",
            "view.set",
            "pen.input",
            "brush.stroke",
            "history.undo"
        ]
    );
    for &s in &path[..5] {
        w.pen(s);
    }
    w.ok(
        "mesh.translate",
        json!({ "direction": [0, 1, 0], "distance": 0.5 }),
    );
    for &s in &path[5..] {
        w.pen(s);
    }
    let steps = w.ok("history.list", json!({}))["steps"].clone();
    let names: Vec<&str> = (steps.as_array().into_iter().flatten())
        .filter_map(|s| s["command"].as_str())
        .collect();
    assert!(
        names.starts_with(&["mesh.translate", "brush.stroke"]),
        "newest first: the edit, after the stroke: {names:?}"
    );
}

#[test]
fn the_window_shows_what_the_part_holds_after_undo() {
    let (mut w, _) = with_sphere(None);
    w.ok("history.checkpoint", json!({ "name": "base" }));
    let base = w.view_pixels();
    w.ok(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 0.3 }),
    );
    let moved = w.view_pixels();
    // Compared with assert!, not assert_eq!: a failure would print millions of bytes.
    assert!(moved != base, "the window follows an edit");
    w.ok("history.restore", json!({ "name": "base" }));
    assert!(w.view_pixels() == base, "a restored part looks as it did");
    w.ok("history.undo", json!({}));
    assert!(
        w.view_pixels() == moved,
        "and undoing the restore brings the edit back"
    );
}

#[test]
fn the_barrel_button_or_the_palette_keep_the_pen_from_sculpting() {
    let (mut w, rect) = with_sphere(Some(log_path("barrel")));
    let before = w.hash();
    for mut s in across(rect, 4) {
        s.buttons = 1;
        w.pen(s);
    }
    assert_eq!(w.hash(), before, "the barrel button navigates");
    w.ok("ui.palette", json!({ "query": "sphere" }));
    for s in across(rect, 4) {
        w.pen(s);
    }
    // Nor anywhere else on the view while it is open, away from the palette itself.
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let palette = panels.iter().find(|p| p["id"] == "palette").map(|p| {
        let r: [f64; 4] = std::array::from_fn(|i| p["rect"][i].as_f64().unwrap_or_default());
        r
    });
    let (x, y) = (rect[0] + 10.0, rect[3] - 10.0);
    let away = palette.is_some_and(|r| {
        let (x, y) = (f64::from(x), f64::from(y));
        x < r[0] || x > r[2] || y < r[1] || y > r[3]
    });
    assert!(
        away,
        "the palette is open and ({x}, {y}) is off it: {palette:?}"
    );
    for (t, dx, pressure) in [
        (20, 0.0, 0.0),
        (24, 0.0, 0.6),
        (28, 20.0, 0.6),
        (32, 20.0, 0.0),
    ] {
        w.pen(pen_at(t, x + dx, y, pressure));
    }
    assert_eq!(w.hash(), before, "the palette is open");
    assert!(
        w.logged().iter().all(|l| l.0 != "brush.stroke"),
        "no stroke"
    );
}

/// Only the active part takes a stroke (decision 0033, section 4), as a stroke over the lids
/// must not dent the eyeball in front of them: with the sphere active and the cube in front of
/// it, a stroke through the cube sculpts the sphere and leaves the cube as it was.
#[test]
fn only_the_active_part_takes_the_stroke_even_behind_another() {
    let mut w = Window::new(Some(log_path("front")));
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    w.ok(
        "primitive.cube",
        json!({ "center": [0.0, 0.0, 1.6], "size": 0.6 }),
    );
    w.ok("part.select", json!({ "part": "sphere" }));
    let get = |w: &mut Window, part: &str| w.ok("part.get", json!({ "part": part }));
    let (sphere, cube) = (get(&mut w, "sphere"), get(&mut w, "cube"));
    let rect = w.view_rect();
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    assert_eq!(
        w.picked(cx, cy)["hit"]["part"],
        json!("cube"),
        "the cube is in front"
    );
    for (t, x, p) in [
        (0, cx, 0.0),
        (4, cx, 0.7),
        (8, cx + 4.0, 0.7),
        (12, cx + 4.0, 0.0),
    ] {
        w.pen(pen_at(t, x, cy, p));
    }
    let stroke = w.logged().last().map(|l| l.1.clone()).unwrap_or_default();
    assert_eq!(stroke["part"], json!("sphere"), "{stroke}");
    assert_eq!(get(&mut w, "cube"), cube, "the cube in front is untouched");
    assert_ne!(
        get(&mut w, "sphere"),
        sphere,
        "the sphere behind it changed"
    );
}

/// A left mouse drag from `from` to `to` in four moves, through the window's loop: the mouse's
/// held buttons become pen samples (`MousePen`, as `app.rs` does), the samples go to
/// `pen.input`, the rest to the studio.
fn mouse_drag(w: &mut Window, from: [f64; 2], to: [f64; 2]) {
    let mut mouse = MousePen::default();
    let at = |k: f64| {
        let [x, y] = [0, 1].map(|i| from[i] + (to[i] - from[i]) * k / 4.0);
        Input::MouseMove { x, y }
    };
    let button = |down, [x, y]: [f64; 2]| Input::MouseButton {
        button: Button::Left,
        down,
        x,
        y,
    };
    let pumps = [
        vec![at(0.0), button(true, from)],
        vec![at(1.0)],
        vec![at(2.0)],
        vec![at(3.0)],
        vec![at(4.0), button(false, to)],
    ];
    for (k, inputs) in pumps.iter().enumerate() {
        let (samples, others) = split_pen(&mouse.convert(inputs, 4_000_000 * k as u64));
        if !samples.is_empty() {
            w.ok("pen.input", json!({ "samples": samples }));
        }
        w.input(&others, Modifiers::default());
    }
}

#[test]
fn a_left_mouse_drag_sculpts_on_the_model_and_turns_the_view_on_empty_canvas() {
    let (mut w, rect) = with_sphere(Some(log_path("mouse")));
    let camera = |w: &mut Window| w.ok("view.get", json!({ "view": "main" }))["camera"].clone();
    let (hash, view) = (w.hash(), camera(&mut w));
    let [cx, cy] = [0, 1].map(|i| f64::from(f32::midpoint(rect[i], rect[i + 2])));
    mouse_drag(&mut w, [cx - 40.0, cy], [cx + 40.0, cy]);
    assert_ne!(w.hash(), hash, "on the model: it sculpted");
    assert_eq!(camera(&mut w), view, "and the view stayed");
    let stroke = w.logged().into_iter().rfind(|l| l.0 == "brush.stroke");
    let samples = stroke.map(|l| l.1["samples"].clone()).unwrap_or_default();
    let first = &samples[0];
    // A sample leaves out a tilt of 0.
    assert_eq!(
        (&first["pressure"], &first["tilt_x"], &first["tilt_y"]),
        (&json!(1.0), &Value::Null, &Value::Null),
        "pressure 1, no tilt: {first}"
    );
    let hash = w.hash();
    let corner = [f64::from(rect[0]) + 30.0, f64::from(rect[3]) - 30.0];
    mouse_drag(&mut w, corner, [corner[0] + 80.0, corner[1]]);
    assert_ne!(camera(&mut w), view, "empty canvas: the view turned");
    assert_eq!(w.hash(), hash, "and nothing was sculpted");
    let pen = w.host.lock().shell.cursor.pen;
    assert!(
        pen.is_some_and(|p| p.in_proximity && !p.contact),
        "the mouse hovers where its button let go, as a pen does: {pen:?}"
    );
}

/// The smoothing string's rectangle in the 3D view, in window pixels, while it shows.
fn string(w: &mut Window) -> Option<[f64; 4]> {
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let view = panels.iter().find(|p| p["id"] == "viewport")?;
    let widgets = view["widgets"].as_array()?;
    let s = widgets.iter().find(|x| x["label"] == "Smoothing string")?;
    Some(std::array::from_fn(|i| {
        s["rect"][i].as_f64().unwrap_or_default()
    }))
}

#[test]
fn the_smoothing_string_shows_from_the_pen_to_the_dab_only_while_a_smoothed_stroke_runs() {
    let (mut w, rect) = with_sphere(None);
    let stroke = across(rect, 12);
    for s in &stroke {
        w.pen(*s);
        assert_eq!(string(&mut w), None, "Draw has no string");
    }
    w.ok(
        "brush.set",
        json!({ "settings": { "stroke": { "smoothing": { "radius": 0.5 } } } }),
    );
    let (lift, down) = stroke.split_last().unwrap_or_else(|| panic!("a stroke"));
    let seen: Vec<_> = (down.iter())
        .map(|s| {
            w.pen(*s);
            string(&mut w)
        })
        .collect();
    assert_eq!(seen[0], None, "not while the pen hovers");
    assert!(seen[1..].iter().all(Option::is_some), "{seen:?}");
    // Draw's radius is 64 pixels, so the string is 32: once the pen has moved farther, the
    // dab trails it by that much, nearly level, and the dot at each end adds 3.
    let r = seen.last().copied().flatten().unwrap_or_default();
    let wide = r[2] - r[0];
    assert!((wide - 38.0).abs() < 1.0, "32 and two dots: {r:?}");
    assert!(
        (r[2] - 3.0 - lift.x).abs() < 0.5,
        "the pen at the right end, the dab behind it: {r:?}"
    );
    w.pen(*lift);
    assert_eq!(string(&mut w), None, "gone when the pen lifts");
}
