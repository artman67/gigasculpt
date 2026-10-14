//! The brush wheel picks any brush in one gesture: hold F2, move toward a slot, let go. It
//! rings the brushes in `brush.list`'s order, the first straight up and the rest
//! clockwise; letting go in the middle picks nothing; a click on a slot picks it; the barrel
//! button opens it; Escape closes it; the middle and the backtick go back to the brush before.
//! It opens a little away from the pen hand, mirrored with the hand; a letter jumps to a slot
//! starting with it; a right click on a slot or `ui.wheel`'s `slot` puts a brush there, and
//! `restore` puts the layout's own back.

use gs_platform::{Button, Input, Modifiers};
use gs_protocol::ErrorCode;
use serde_json::{Value, json};

use crate::rig::{Ui, center, key};

/// No modifier held.
const NONE: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
    logo: false,
};

/// The wheel's slots: label and middle, in pixels.
fn slots(ui: &mut Ui) -> Vec<(String, [f64; 2])> {
    let widgets = ui.widgets("wheel");
    (widgets.iter())
        .filter(|w| w["command"]["args"].get("brush").is_some())
        .map(|w| {
            (
                w["label"].as_str().unwrap_or_default().to_string(),
                center(&w["rect"]),
            )
        })
        .collect()
}

/// What picking the brush `label` sets, as `session.state` names it, and the command its slot
/// runs: a Smooth brush goes on Shift's Smooth slot (brush-settings.modifier-key-slots), any
/// other becomes the brush. `listed` is `brush.list`'s brushes.
fn picks(listed: &Value, label: &str) -> (&'static str, &'static str) {
    let mut all = listed.as_array().into_iter().flatten();
    if all.any(|b| b["name"] == label && b["family"] == "Smooth") {
        ("brush.slot.smooth", "brush.slot")
    } else {
        ("brush", "brush.set")
    }
}

/// F2 down with the pen at `from`, the pen moved to `to`, F2 up.
fn flick(ui: &mut Ui, from: [f64; 2], to: [f64; 2]) {
    ui.pen(from[0], from[1]);
    ui.input(&[key("F2", true, NONE)], NONE);
    ui.pen(to[0], to[1]);
    ui.input(&[key("F2", false, NONE)], NONE);
}

#[test]
fn a_flick_toward_any_slot_picks_its_brush() {
    let mut ui = Ui::new();
    let listed = ui.ok("brush.list", json!({}))["brushes"].clone();
    let brushes: Vec<&str> = (listed.as_array().into_iter().flatten())
        .filter_map(|b| b["name"].as_str())
        .collect();
    let at = ui.middle();
    ui.pen(at[0], at[1]);
    ui.input(&[key("F2", true, NONE)], NONE);
    let all = slots(&mut ui);
    let labels: Vec<&str> = all.iter().map(|s| s.0.as_str()).collect();
    assert_eq!(labels, brushes, "one ring of every brush, in order");
    // Every slot runs brush.set, or brush.slot for a Smooth brush.
    let runs: Vec<(Value, Value)> = (ui.widgets("wheel").iter())
        .filter_map(|w| w.get("command").filter(|c| !c.is_null()))
        .map(|c| (c["command"].clone(), c["args"]["brush"].clone()))
        .collect();
    assert!(!runs.is_empty(), "the slots run commands");
    for (command, brush) in &runs {
        let wants = brush.as_str().map_or("brush.set", |b| picks(&listed, b).1);
        assert_eq!(command, wants, "the slot for {brush}");
    }
    let wheel = ui.describe()["panels"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["id"] == "wheel")
        .cloned()
        .unwrap_or_default();
    let r = |i: usize| wheel["rect"][i].as_f64().unwrap_or_default();
    for (label, [x, y]) in &all {
        assert!(
            r(0) < *x && *x < r(2) && r(1) < *y && *y < r(3),
            "{label} inside the wheel"
        );
    }
    let top = all
        .iter()
        .min_by(|a, b| a.1[1].total_cmp(&b.1[1]))
        .map(|s| s.0.clone());
    assert_eq!(
        top.as_deref(),
        brushes.first().copied(),
        "the first brush straight up"
    );
    ui.input(&[key("F2", false, NONE)], NONE);
    for (label, to) in all {
        flick(&mut ui, at, to);
        let picked = picks(&listed, &label).0;
        assert_eq!(ui.state()[picked], json!(label), "flicked to {label}");
        assert!(ui.widgets("wheel").is_empty(), "letting go closes it");
    }
}

#[test]
fn the_middle_picks_nothing_clicks_pick_and_the_brush_before_is_one_press_away() {
    let mut ui = Ui::new();
    let at = ui.middle();
    flick(&mut ui, at, [at[0] + 10.0, at[1] + 10.0]);
    assert_eq!(
        ui.state()["brush"],
        json!("Draw"),
        "let go in the middle: nothing picked"
    );
    // Pointing lights the slot up and says what letting go picks.
    ui.pen(at[0], at[1]);
    ui.input(&[key("F2", true, NONE)], NONE);
    let pinch = slots(&mut ui)
        .into_iter()
        .find(|s| s.0 == "Pinch")
        .map(|s| s.1)
        .unwrap_or_default();
    ui.pen(pinch[0], pinch[1]);
    let labels = ui.labels("wheel");
    assert!(labels.iter().any(|l| l == "Let go for Pinch"), "{labels:?}");
    ui.input(&[key("Escape", true, NONE)], NONE);
    ui.input(&[key("F2", false, NONE)], NONE);
    assert_eq!(ui.state()["brush"], json!("Draw"), "Escape closed it first");
    // The barrel button's click opens it at the pen; a tap on a slot picks it.
    let barrel = |ui: &mut Ui, down: bool| {
        let sample = json!({ "samples": [{ "x": at[0], "y": at[1], "buttons": u32::from(down) }] });
        ui.ok("pen.input", sample);
        ui.input(&[], NONE);
    };
    barrel(&mut ui, true);
    barrel(&mut ui, false);
    let trim = slots(&mut ui)
        .into_iter()
        .find(|s| s.0 == "Trim")
        .map(|s| s.1)
        .unwrap_or_default();
    ui.touch(trim[0], trim[1], false);
    ui.touch(trim[0], trim[1], true);
    ui.touch(trim[0], trim[1], false);
    ui.input(&[], NONE);
    assert_eq!(ui.state()["brush"], json!("Trim"), "tapped");
    assert!(ui.widgets("wheel").is_empty(), "and closed");
    // Back to the brush before: the backtick, then the wheel's middle.
    ui.tap("Backtick", NONE);
    assert_eq!(ui.state()["brush"], json!("Draw"));
    ui.pen(at[0], at[1]);
    ui.ok("ui.wheel", json!({ "open": true }));
    let back: Value = ui
        .widgets("wheel")
        .into_iter()
        .find(|w| w["label"] == "Back to Trim")
        .unwrap_or_default();
    let [x, y] = center(&back["rect"]);
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x, y, false);
    ui.input(&[], NONE);
    assert_eq!(ui.state()["brush"], json!("Trim"), "the middle goes back");
}

/// The open wheel's middle, in pixels.
fn wheel_middle(ui: &mut Ui) -> [f64; 2] {
    let panels = ui.describe()["panels"].clone();
    let wheel = (panels.as_array().into_iter().flatten()).find(|p| p["id"] == "wheel");
    center(&wheel.unwrap_or_else(|| panic!("the wheel is open"))["rect"])
}

/// The wheel opened with the pen in the 3D view's middle: where the pen is and where the
/// wheel's middle is.
fn open_at_middle(ui: &mut Ui) -> ([f64; 2], [f64; 2]) {
    let at = ui.middle();
    ui.pen(at[0], at[1]);
    ui.ok("ui.wheel", json!({ "open": true }));
    (at, wheel_middle(ui))
}

#[test]
fn the_wheel_opens_away_from_the_pen_hand_and_the_mirror_moves_it() {
    let mut ui = Ui::new();
    let offset = ui.ok("ui.settings", json!({}))["wheel_offset"].as_f64();
    assert_eq!(offset, Some(24.0), "24 points at first");
    // How far right of the pen the wheel's middle is, and how far below.
    let moved = |(pen, middle): ([f64; 2], [f64; 2])| {
        [middle[0] - pen[0], middle[1] - pen[1]].map(|d| (d * 10.0).round() / 10.0)
    };
    let right = moved(open_at_middle(&mut ui));
    assert_eq!(right, [-24.0, 0.0], "right hand: to the pen's left");
    let picked = ui.ok("ui.wheel", json!({ "open": false }))["picked"].clone();
    assert_eq!(
        picked,
        Value::Null,
        "letting go where it opened picks nothing"
    );
    ui.ok("ui.layout", json!({ "hand": "left" }));
    let left = moved(open_at_middle(&mut ui));
    assert_eq!(left, [24.0, 0.0], "left hand: mirrored to its right");
    ui.ok("ui.wheel", json!({ "open": false }));
    ui.ok("ui.settings", json!({ "wheel_offset": 0 }));
    let none = moved(open_at_middle(&mut ui));
    assert_eq!(none, [0.0, 0.0], "no offset: at the pen");
    let (code, _) = ui.err("ui.settings", json!({ "wheel_offset": 60 }));
    assert_eq!(code, ErrorCode::ArgsInvalid, "past the middle's edge");
}

#[test]
fn a_letter_jumps_to_the_next_slot_starting_with_it_and_letting_go_picks_it() {
    let mut ui = Ui::new();
    let listed = ui.ok("brush.list", json!({}))["brushes"].clone();
    let at = ui.middle();
    ui.pen(at[0], at[1]);
    ui.input(&[key("F2", true, NONE)], NONE);
    let starting = |s: &(String, [f64; 2])| s.0.starts_with('S');
    let s: Vec<String> = slots(&mut ui)
        .into_iter()
        .filter(starting)
        .map(|s| s.0)
        .collect();
    assert!(s.len() > 1, "two brushes start with S: {s:?}");
    let (picked, command) = picks(&listed, &s[1]);
    let before = ui.logged(command).len();
    ui.tap("S", NONE);
    let lit = format!("Let go for {}", s[0]);
    assert!(ui.labels("wheel").contains(&lit), "S jumps to {}", s[0]);
    ui.tap("S", NONE);
    let lit = format!("Let go for {}", s[1]);
    assert!(ui.labels("wheel").contains(&lit), "again: {}", s[1]);
    ui.input(&[key("F2", false, NONE)], NONE);
    assert_eq!(ui.state()[picked], json!(s[1]), "the pen never moved");
    let runs = ui.logged(command).len() - before;
    assert_eq!(runs, 1, "S went to the wheel, not to the Size key");
}

#[test]
fn a_right_click_on_a_slot_puts_the_brush_in_use_there() {
    let mut ui = Ui::new();
    ui.ok("brush.set", json!({ "brush": "Pinch" }));
    open_at_middle(&mut ui);
    let first = slots(&mut ui)[0].clone();
    assert_ne!(first.0, "Pinch");
    let [x, y] = first.1;
    let right = |down| Input::MouseButton {
        button: Button::Right,
        down,
        x,
        y,
    };
    ui.input(&[Input::MouseMove { x, y }, right(true)], NONE);
    ui.input(&[right(false)], NONE);
    assert_eq!(slots(&mut ui)[0].0, "Pinch", "the top slot holds Pinch");
    assert_eq!(ui.state()["brush"], json!("Pinch"), "nothing picked");
    // The command a right click runs, from an agent; the layout state keeps the ring.
    let info = ui.ok("ui.wheel", json!({ "slot": 1, "brush": "Trim" }));
    assert_eq!(info["slots"][0], json!("Pinch"));
    assert_eq!(info["slots"][1], json!("Trim"));
    assert!(info["open"].as_bool().unwrap_or_default(), "still open");
    let ring = ui.host.lock().shell.layout.layout.wheel.clone();
    assert_eq!(ring.get(..2), Some(&["Pinch".into(), "Trim".into()][..]));
    let refused = [
        json!({ "slot": 99, "brush": "Trim" }),
        json!({ "slot": 0, "brush": "No such brush" }),
    ];
    for args in refused {
        assert_eq!(ui.err("ui.wheel", args).0, ErrorCode::NotFound);
    }
    let (code, _) = ui.err("ui.wheel", json!({ "slot": 0 }));
    assert_eq!(code, ErrorCode::ArgsInvalid, "a slot needs a brush");
    let back = ui.ok("ui.wheel", json!({ "restore": true }))["slots"].clone();
    let listed = ui.ok("brush.list", json!({}))["brushes"].clone();
    let names: Vec<Value> = (listed.as_array().into_iter().flatten())
        .map(|b| b["name"].clone())
        .collect();
    assert_eq!(back, json!(names), "Restore puts the layout's ring back");
}
