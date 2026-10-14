//! The default sculpting keys, pressed through the studio as the window's keyboard sends them:
//! B and two letters pick a brush, X turns X symmetry on and off, [ and ] step the size, and S
//! or U held past the activation time while the pointer moves sideways set size or strength
//! once, when let go (sticky keys); a number typed while one is held sets it exactly, and a
//! short tap shows the slider. The view keys act on the view under the pointer: Period frames
//! the last edit and V H levels the view, the key the View menu's Level row shows. The sculpt
//! table's Check (`interface.default-bindings-sculpt`) runs as written.

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Window, across, pen_at, with_sphere};

fn key(name: &str, down: bool) -> Input {
    Input::Key {
        key: name.into(),
        down,
        repeat: false,
        modifiers: Modifiers::default(),
    }
}

/// Presses and lets go of each key in turn, one frame each.
fn tap(w: &mut Window, keys: &[&str]) {
    for k in keys {
        w.input(&[key(k, true)], Modifiers::default());
        w.input(&[key(k, false)], Modifiers::default());
    }
}

/// The session's state values.
fn current(w: &mut Window) -> Value {
    w.ok("session.state", json!({}))
}

/// Presses `name` and holds it past the activation time, a quarter second by default.
fn hold(w: &mut Window, name: &str) {
    w.input(&[key(name, true)], Modifiers::default());
    w.wait(0.3);
}

/// Whether the quick menu is open.
fn quick_open(w: &mut Window) -> bool {
    let panels = w.ok("ui.describe", json!({}))["panels"].clone();
    (panels.as_array().into_iter().flatten()).any(|p| p["id"] == "quick")
}

#[test]
fn letters_after_b_pick_the_brush() {
    let mut w = Window::new(None);
    assert_eq!(current(&mut w)["brush"], json!("Draw"), "Draw to start");
    for (keys, brush) in [
        (["B", "C", "B"], "Clay Build"),
        (["B", "S", "S"], "Smooth Strong"),
        (["B", "M", "V"], "Move"),
        (["B", "S", "T"], "Draw"),
    ] {
        tap(&mut w, &keys);
        assert_eq!(current(&mut w)["brush"], json!(brush), "{keys:?}");
    }
    // A key that ends no sequence starts one of its own (the second B), or none (Q).
    tap(&mut w, &["B", "Q", "B", "B", "C", "C"]);
    assert_eq!(current(&mut w)["brush"], json!("Clip Curve"));
}

#[test]
fn x_flips_symmetry_and_brackets_step_the_size() {
    let mut w = Window::new(None);
    tap(&mut w, &["X"]);
    assert_eq!(current(&mut w)["symmetry.x"], json!(true));
    tap(&mut w, &["X"]);
    assert_eq!(current(&mut w)["symmetry.x"], json!(false));
    tap(&mut w, &["]", "]"]);
    assert_eq!(
        current(&mut w)["brush.size_px"],
        json!(80.0),
        "64 and two steps of 8"
    );
    tap(&mut w, &["["]);
    assert_eq!(current(&mut w)["brush.size_px"], json!(72.0));
}

#[test]
fn a_held_key_sets_its_value_with_the_pointer() {
    // Two pixels per point: the size follows the pointer pixel for pixel.
    let mut w = Window::with(None, gs_ui::Keymap::default(), 2.0);
    // The pen hovers over the middle of the 3D view, so its cursor shows the brush.
    let r = w.view_rect();
    let middle = (f32::midpoint(r[0], r[2]), f32::midpoint(r[1], r[3]));
    w.pen(pen_at(0, middle.0, middle.1, 0.0));
    let at = |x: f64| Input::MouseMove { x, y: 200.0 };
    w.input(&[at(200.0)], Modifiers::default());
    hold(&mut w, "S");
    w.input(&[at(300.0)], Modifiers::default());
    assert_eq!(
        current(&mut w)["brush.size_px"],
        json!(64.0),
        "nothing runs while it is held"
    );
    let shown = w.brush_label().map(|l| l.0);
    assert_eq!(
        shown.as_deref(),
        Some("Draw  164 px  40%"),
        "the cursor shows the size as it is set"
    );
    // Other keys wait while one is held.
    tap(&mut w, &["X"]);
    w.input(&[at(260.0), key("S", false)], Modifiers::default());
    let now = current(&mut w);
    assert_eq!(
        now["brush.size_px"],
        json!(124.0),
        "60 pixels to the right, 60 more: {now}"
    );
    assert_eq!(now["symmetry.x"], json!(false), "X waited: {now}");
    hold(&mut w, "S");
    w.input(&[at(-1000.0), key("S", false)], Modifiers::default());
    assert_eq!(
        current(&mut w)["brush.size_px"],
        json!(1.0),
        "never below a pixel"
    );
    // Strength: 400 points (800 pixels here) span all of it; it stops at 1; Escape lets go
    // without setting it.
    w.input(&[at(0.0)], Modifiers::default());
    hold(&mut w, "U");
    w.input(&[at(200.0)], Modifiers::default());
    let shown = w.brush_label().map(|l| l.0);
    assert_eq!(
        shown.as_deref(),
        Some("Draw  1 px  65%"),
        "and the strength"
    );
    w.input(&[key("U", false)], Modifiers::default());
    let strength = current(&mut w)["brush.strength"]
        .as_f64()
        .unwrap_or_default();
    assert!(
        (strength - 0.65).abs() < 1e-6,
        "Draw's 0.4 and a quarter more for 100 points: {strength}"
    );
    hold(&mut w, "U");
    w.input(&[at(900.0), key("U", false)], Modifiers::default());
    assert_eq!(
        current(&mut w)["brush.strength"],
        json!(1.0),
        "never above 1"
    );
    hold(&mut w, "U");
    w.input(&[at(30.0), key("Escape", true)], Modifiers::default());
    w.input(&[key("U", false)], Modifiers::default());
    assert_eq!(
        current(&mut w)["brush.strength"],
        json!(1.0),
        "Escape keeps the strength"
    );
}

#[test]
fn any_argument_can_be_held_and_set() {
    let mut keymap = gs_ui::Keymap::default();
    let args = toml::from_str("modifier = \"drag\"").unwrap_or_else(|e| panic!("{e}"));
    keymap.bindings.push(gs_ui::Binding {
        key: "M".into(),
        command: "brush.set".into(),
        args,
    });
    let mut w = Window::with(None, keymap, 1.0);
    let at = |x: f64| Input::MouseMove { x, y: 200.0 };
    w.input(&[at(100.0)], Modifiers::default());
    hold(&mut w, "M");
    w.input(&[at(150.0), key("M", false)], Modifiers::default());
    let modifier = current(&mut w)["brush.modifier"]
        .as_f64()
        .unwrap_or_default();
    assert!(
        (modifier - 0.5).abs() < 1e-6,
        "a hundredth per point: {modifier}"
    );
}

/// A number typed while a slider's key is held sets it exactly, as the window writes it:
/// the size in pixels, the strength in percent. It need not wait for the activation time, and
/// it wins over the pointer.
#[test]
fn a_number_typed_while_a_slider_key_is_held_sets_it_exactly() {
    let mut w = Window::new(None);
    // The 3D view's middle row: in this small window the header's rows reach past row 200.
    let r = w.view_rect();
    let y = f32::midpoint(r[1], r[3]);
    let at = |x: f64| Input::MouseMove { x, y: y.into() };
    w.input(&[at(200.0)], Modifiers::default());
    w.input(&[key("S", true)], Modifiers::default());
    tap(&mut w, &["8", "0"]);
    w.pen(pen_at(0, f32::midpoint(r[0], r[2]), y, 0.0));
    let shown = w.brush_label().map(|l| l.0);
    assert_eq!(shown.as_deref(), Some("Draw  80 px  40%"), "shown as typed");
    w.input(&[key("S", false)], Modifiers::default());
    assert_eq!(
        current(&mut w)["brush.size_px"],
        json!(80.0),
        "set as typed"
    );
    hold(&mut w, "U");
    w.input(&[at(300.0)], Modifiers::default());
    tap(&mut w, &["7", "2", "Backspace", "5"]);
    w.input(&[at(400.0), key("U", false)], Modifiers::default());
    let strength = current(&mut w)["brush.strength"]
        .as_f64()
        .unwrap_or_default();
    assert!(
        (strength - 0.75).abs() < 1e-6,
        "75%, not the pointer's: {strength}"
    );
    assert!(!quick_open(&mut w), "a typed number shows no slider");
}

/// Sticky keys (on by default): a slider's key held past the activation time and moved 80
/// pixels changes that slider, a move before that time does not count, and a short tap only
/// shows the slider (alone at the pointer, in the quick menu's window). Off, a held slider key
/// only shows the slider when let go: the same movement changes nothing, and a number typed
/// while it is held still sets it.
#[test]
fn sticky_keys_set_a_slider_only_once_held_and_a_tap_shows_it() {
    let mut w = Window::new(None);
    let size = |w: &mut Window| current(w)["brush.size_px"].clone();
    let at = |x: f64| Input::MouseMove { x, y: 200.0 };
    assert_eq!(current(&mut w)["settings.sticky_keys"], json!(true));
    w.input(&[at(100.0)], Modifiers::default());
    hold(&mut w, "S");
    w.input(&[at(180.0), key("S", false)], Modifiers::default());
    assert_eq!(size(&mut w), json!(144.0), "64 and 80");
    assert!(!quick_open(&mut w), "a hold shows no slider");
    w.input(&[key("S", true)], Modifiers::default());
    w.input(&[at(260.0)], Modifiers::default());
    w.wait(0.3);
    w.input(&[key("S", false)], Modifiers::default());
    assert_eq!(
        size(&mut w),
        json!(144.0),
        "moved before the time: no change"
    );
    tap(&mut w, &["S"]);
    assert_eq!(size(&mut w), json!(144.0), "a tap changes nothing");
    assert!(quick_open(&mut w), "a tap shows the slider");
    tap(&mut w, &["Escape"]);
    assert!(!quick_open(&mut w));
    w.ok("ui.settings", json!({ "sticky_keys": false }));
    hold(&mut w, "S");
    w.input(&[at(340.0), key("S", false)], Modifiers::default());
    assert_eq!(
        size(&mut w),
        json!(144.0),
        "off: the movement changes nothing"
    );
    assert!(quick_open(&mut w), "off: the key shows the slider");
    // Numeric entry is a setting of its own: off, a number typed while the key is held still
    // sets it exactly.
    tap(&mut w, &["Escape"]);
    w.input(&[key("S", true)], Modifiers::default());
    tap(&mut w, &["8", "0"]);
    w.input(&[key("S", false)], Modifiers::default());
    assert_eq!(size(&mut w), json!(80.0), "off: typed");
    assert!(!quick_open(&mut w), "off: a typed number shows no slider");
}

#[test]
fn shift_f_switches_the_wireframe_of_the_view_under_the_pointer_only() {
    let (mut w, _) = with_sphere(None);
    w.ok("reference.open", json!({ "scene": true }));
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    let shift_f_at = |w: &mut Window, at: [f64; 2]| {
        let to = Input::MouseMove { x: at[0], y: at[1] };
        w.input(&[to], Modifiers::default());
        let f = |down| Input::Key {
            key: "F".into(),
            down,
            repeat: false,
            modifiers: shift,
        };
        w.input(&[f(true)], shift);
        w.input(&[f(false)], shift);
    };
    let wires = |w: &mut Window| {
        let of = |w: &mut Window, view: &str| {
            w.ok("view.get", json!({ "view": view }))["wireframe"].clone()
        };
        (of(w, "main"), of(w, "ref:1"))
    };
    let reference = middle(w.rect("reference > ref:1"));
    let view = middle(w.rect("viewport"));
    shift_f_at(&mut w, reference);
    assert_eq!(
        wires(&mut w),
        (json!(false), json!(true)),
        "over the reference"
    );
    shift_f_at(&mut w, view);
    assert_eq!(
        wires(&mut w),
        (json!(true), json!(true)),
        "over the 3D view"
    );
    let d = w.ok("ui.describe", json!({}));
    // The header's Wire, not the Reference View bar's, which names its view.
    let wire = (d["panels"].as_array().into_iter().flatten())
        .flat_map(|p| p["widgets"].as_array().into_iter().flatten())
        .find(|x| {
            let step = &x["command"];
            x["label"] == "Wire" && step["command"] == "view.look" && step["args"]["view"].is_null()
        })
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        (&wire["selected"], &wire["key"]),
        (&json!(true), &json!("Shift+F")),
        "the header's Wire lights, with its key"
    );
    shift_f_at(&mut w, reference);
    assert_eq!(
        wires(&mut w),
        (json!(true), json!(false)),
        "the reference alone again"
    );
}

#[test]
fn period_frames_the_last_edit_and_v_h_levels_the_view() {
    let (mut w, rect) = with_sphere(None);
    for s in across(rect, 12) {
        w.pen(s);
    }
    let camera = |w: &mut Window| w.ok("view.get", json!({ "view": "main" }))["camera"].clone();
    let over = middle(w.rect("viewport"));
    w.input(
        &[Input::MouseMove {
            x: over[0],
            y: over[1],
        }],
        Modifiers::default(),
    );
    let before = camera(&mut w);
    tap(&mut w, &["."]);
    let framed = camera(&mut w);
    assert_ne!(framed, before, "Period framed something");
    // The same stroke in another window, framed by the command Frame last edit runs.
    let (mut twin, _) = with_sphere(None);
    for s in across(rect, 12) {
        twin.pen(s);
    }
    twin.ok("view.frame", json!({ "target": "last-edit" }));
    assert_eq!(
        camera(&mut twin),
        framed,
        "the last edit, as Frame last edit frames it"
    );
    w.ok("view.roll", json!({ "deg": 30 }));
    tap(&mut w, &["V", "H"]);
    let level = w.ok("view.get", json!({ "view": "main" }));
    let exact: gs_engine::ExactCamera =
        serde_json::from_value(level["camera"].clone()).unwrap_or_else(|e| panic!("{e}"));
    let (right, up, _) = exact
        .camera("")
        .unwrap_or_else(|e| panic!("{}", e.message))
        .axes();
    assert!(
        right.y.abs() < 1e-5 && up.y > 0.0,
        "V H took the roll away: right {right:?}, up {up:?}"
    );
    w.ok("ui.menu", json!({ "open": "view-menu" }));
    let d = w.ok("ui.describe", json!({}));
    let button = (d["panels"].as_array().into_iter().flatten())
        .flat_map(|p| p["widgets"].as_array().into_iter().flatten())
        .find(|x| x["label"] == "Level" && x["command"]["command"] == "view.level")
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        button["key"],
        json!("V H"),
        "the View menu's Level row shows its key"
    );
}

#[test]
fn l_turns_the_smoothing_string_off_and_back_to_its_length_and_comma_flips_the_library() {
    let mut w = Window::new(Some(crate::rig::log_path("l-key")));
    let smoothing = |w: &mut Window| current(w)["brush.smoothing"].clone();
    assert_eq!(smoothing(&mut w), json!(0.0), "Draw has no string");
    tap(&mut w, &["L"]);
    assert_eq!(smoothing(&mut w), json!(1.0), "on, a brush radius long");
    tap(&mut w, &["L"]);
    assert_eq!(smoothing(&mut w), json!(0.0), "off");
    w.ok(
        "brush.set",
        json!({ "settings": { "stroke": { "smoothing": { "radius": 2.5 } } } }),
    );
    tap(&mut w, &["L", "L"]);
    assert_eq!(
        smoothing(&mut w),
        json!(2.5),
        "off and on: the length it had"
    );
    // L runs the same `brush.set` the Smoothing slider does (decision 0036 section 6).
    let set = (w.logged().into_iter())
        .filter(|l| l.0 == "brush.set")
        .map(|l| l.1["settings"]["stroke"]["smoothing"]["radius"].clone());
    assert_eq!(
        set.collect::<Vec<_>>(),
        [1.0, 0.0, 2.5, 0.0, 2.5].map(|r| json!(r)),
        "each in the slider's argument"
    );
    let library = |w: &mut Window| current(w)["layout.library"].clone();
    let shut = library(&mut w);
    tap(&mut w, &["Comma"]);
    assert_ne!(library(&mut w), shut, "Comma opens the Library");
    tap(&mut w, &["Comma"]);
    assert_eq!(library(&mut w), shut, "and shuts it");
}

/// Catalog `interface.default-bindings-sculpt`, its Check as written: "B, S, T selects Draw and
/// B, S, M selects Smooth; S held while the pointer moves right makes the brush larger; L turns
/// stroke smoothing off and, pressed again, back on at the length it had".
#[test]
fn the_sculpt_check_b_s_t_and_b_s_m_pick_s_held_grows_and_l_keeps_the_length() {
    let mut w = Window::new(None);
    tap(&mut w, &["B", "S", "M"]);
    assert_eq!(current(&mut w)["brush"], json!("Smooth"), "B, S, M");
    tap(&mut w, &["B", "S", "T"]);
    assert_eq!(current(&mut w)["brush"], json!("Draw"), "B, S, T");
    let size = |w: &mut Window| current(w)["brush.size_px"].as_f64().unwrap_or_default();
    let before = size(&mut w);
    let at = |x: f64| Input::MouseMove { x, y: 200.0 };
    w.input(&[at(200.0)], Modifiers::default());
    hold(&mut w, "S");
    w.input(&[at(260.0), key("S", false)], Modifiers::default());
    assert!(size(&mut w) > before, "S held moving right: {before} grew");
    let radius = json!({ "settings": { "stroke": { "smoothing": { "radius": 2.5 } } } });
    w.ok("brush.set", radius);
    tap(&mut w, &["L"]);
    assert_eq!(current(&mut w)["brush.smoothing"], json!(0.0), "L: off");
    tap(&mut w, &["L"]);
    assert_eq!(
        current(&mut w)["brush.smoothing"],
        json!(2.5),
        "again: its length"
    );
}
