//! A slider alone (catalog `interface.on-canvas-adjust`, decision 0025 section 2 as amended
//! 2026-10-12): with sticky keys on, a short tap of a slider key shows only that slider, with
//! its knob under the pointer at the value it holds (Win-feel), and Escape or a press outside
//! closes it as the quick menu closes; the wheel steps it while it shows. While it shows, and
//! while a slider key is held, the brush circle stays where the pen was, at the value being
//! set. A key that drags a control's argument shows that control's slider.

use gs_platform::{Input, Modifiers};
use serde_json::json;

use crate::quick::pointer;
use crate::region::middle;
use crate::rig::{Ui, key};

/// The labels in the quick menu's window.
fn shown(ui: &mut Ui) -> Vec<String> {
    ui.labels("quick")
}

/// The value the quick window's Size slider shows, in pixels (while dragged, where the drag
/// has it).
fn size_shown(ui: &mut Ui) -> f64 {
    let widgets = ui.widgets("quick");
    let size = widgets.iter().find(|w| w["label"] == "Size");
    size.map_or(0.0, |w| {
        w["command"]["args"]["size_px"].as_f64().unwrap_or_default()
    })
}

/// Where the knob of the quick window's slider `label` (Size or Strength) is drawn, in pixels,
/// worked out from its row as the slider draws it: the rail is the row's first `slider` points
/// and as tall as the row, and the knob's center runs from a knob's radius (the height over
/// 2.5) in from one end to as far in from the other, Size on a logarithmic scale from 1 to
/// 2000 and Strength from 0 to 1.
fn knob(ui: &mut Ui, label: &str) -> [f64; 2] {
    let widgets = ui.widgets("quick");
    let found = widgets
        .iter()
        .find(|w| w["label"] == label && w["role"] == "slider");
    let w = found
        .cloned()
        .unwrap_or_else(|| panic!("no {label} slider: {widgets:?}"));
    let r: [f64; 4] = std::array::from_fn(|i| w["rect"][i].as_f64().unwrap_or_default());
    let args = &w["command"]["args"];
    let place = if label == "Size" {
        libm::log(args["size_px"].as_f64().unwrap_or_default()) / libm::log(2000.0)
    } else {
        args["strength"].as_f64().unwrap_or_default()
    };
    let rail = f64::from(ui.host.lock().theme().spacing.slider);
    let radius = (r[3] - r[1]) / 2.5;
    let along = place.clamp(0.0, 1.0) * (rail - 2.0 * radius);
    [r[0] + radius + along, f64::midpoint(r[1], r[3])]
}

/// How far apart `a` and `b` are, in pixels.
pub(crate) fn apart(a: [f64; 2], b: [f64; 2]) -> f64 {
    libm::hypot(a[0] - b[0], a[1] - b[1])
}

/// The Size ring the 3D view draws, if it draws one.
fn ring(ui: &mut Ui) -> Option<[f64; 4]> {
    let widgets = ui.widgets("viewport");
    let found = widgets.iter().find(|w| w["label"] == "Size ring")?;
    Some(std::array::from_fn(|i| {
        found["rect"][i].as_f64().unwrap_or_default()
    }))
}

fn width(r: [f64; 4]) -> f64 {
    r[2] - r[0]
}

/// Decision 0025 section 2 (amended 2026-10-12, Win-feel): the maintainer asked that "the knob
/// on the slider should spawn exactly on top of your cursor when you press S so you can change
/// it instantly". At a size mid-rail and near each end, and at the window's left and right
/// edges, where the window runs past the edge rather than move the knob off the pointer.
#[test]
fn the_size_slider_alone_opens_with_its_knob_under_the_pointer() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let [x, y] = ui.middle();
    let right = f64::from(crate::rig::SIZE[0]);
    for (size, at) in [
        (45.0, [x, y]),
        (2.0, [x - 100.0, y + 50.0]),
        (1900.0, [x + 100.0, y - 50.0]),
        (1900.0, [4.0, y]),
        (2.0, [right - 4.0, y]),
    ] {
        ui.ok("brush.set", json!({ "size_px": size }));
        ui.pen(at[0], at[1]);
        ui.tap("S", Modifiers::default());
        assert_eq!(shown(&mut ui).first().map(String::as_str), Some("Size"));
        let knob = knob(&mut ui, "Size");
        assert!(
            apart(knob, at) <= 1.0,
            "Size {size}: the knob at {knob:?}, the pointer at {at:?}"
        );
        ui.tap("Escape", Modifiers::default());
    }
}

/// The maintainer's report: "When you press S to change brush size, your cursor disappears so
/// you can't see how big your brush is becoming while you adjust the slider." While the Size
/// slider shows alone the brush circle stays where the pen was, without the text beside it
/// (which would lie over the slider), and grows as the knob is dragged; the wheel over the knob
/// keeps the slider alone's step.
#[test]
fn while_the_size_slider_shows_alone_the_brush_circle_stays_and_follows_its_knob() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let hovering = ring(&mut ui).unwrap_or_else(|| panic!("the pen's circle"));
    ui.tap("S", Modifiers::default());
    assert_eq!(ui.ok("ui.quick", json!({}))["slider"], json!("Size"));
    assert_eq!(ring(&mut ui), Some(hovering), "the circle stays as it was");
    let labels = ui.labels("viewport");
    assert!(
        !labels.iter().any(|l| l.starts_with("Draw")),
        "no text beside it: {labels:?}"
    );

    // The knob dragged 10 pixels right: the circle grows with the size, around the same spot.
    let start = size_shown(&mut ui);
    let from = knob(&mut ui, "Size");
    ui.touch(from[0], from[1], true);
    ui.touch(from[0] + 10.0, from[1], true);
    ui.settle();
    let dragged = size_shown(&mut ui);
    assert!(
        dragged > start * 1.5,
        "the drag grew it: {start} to {dragged}"
    );
    let grown = ring(&mut ui).unwrap_or_else(|| panic!("the circle while dragging"));
    let ratio = width(grown) / width(hovering);
    assert!(
        (ratio / (dragged / start) - 1.0).abs() < 0.03,
        "the circle grew {ratio} times, the size {} times",
        dragged / start
    );
    let centers = [grown, hovering].map(middle);
    assert!(
        apart(centers[0], centers[1]) <= 2.0,
        "around the same spot: {centers:?}"
    );
    ui.touch(from[0] + 10.0, from[1], false);
    let set = ui.state()["brush.size_px"].as_f64().unwrap_or_default();
    assert!((set - dragged).abs() < 0.5, "let go at {set}");

    // A notch over the knob steps the size as the slider alone's wheel does, by a size step.
    ui.input(&[Input::Wheel { dx: 0.0, dy: 1.0 }], Modifiers::default());
    let stepped = ui.state()["brush.size_px"].as_f64().unwrap_or_default();
    assert!((stepped - set - 8.0).abs() < 1e-3, "{set} to {stepped}");
}

/// The same while a slider key is held, before any slider shows: the circle stays where the pen
/// was when the key went down and grows as the pen moves right; let go, it follows the pen.
#[test]
fn a_held_size_key_keeps_the_brush_circle_where_the_pen_was() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let [x, y] = ui.middle();
    ui.pen(x, y);
    let hovering = ring(&mut ui).unwrap_or_else(|| panic!("the pen's circle"));
    let none = Modifiers::default();
    ui.input(&[key("S", true, none)], none);
    ui.wait(0.3);
    ui.pen(x + 40.0, y);
    let held = ring(&mut ui).unwrap_or_else(|| panic!("the circle while held"));
    let centers = [held, hovering].map(middle);
    assert!(
        apart(centers[0], centers[1]) <= 2.0,
        "where the pen was: {centers:?}"
    );
    assert!(
        width(held) > width(hovering) + 60.0,
        "40 pixels bigger: {} to {}",
        width(hovering),
        width(held)
    );
    ui.input(&[key("S", false, none)], none);
    let after = ring(&mut ui).unwrap_or_else(|| panic!("the circle let go"));
    assert!(
        apart(middle(after), [x + 40.0, y]) <= 4.0,
        "following the pen again: {after:?}"
    );
}

/// Catalog `interface.on-canvas-adjust`: "with sticky keys on ... a short tap only shows the
/// slider".
#[test]
fn a_short_tap_of_a_slider_key_shows_that_slider_alone_at_the_pointer() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    assert_eq!(ui.state()["settings.sticky_keys"], json!(true));
    let at = pointer(&mut ui);
    let strength = |ui: &mut Ui| ui.state()["brush.strength"].as_f64().unwrap_or_default();
    let start = strength(&mut ui);
    ui.tap("U", Modifiers::default());
    assert_eq!(strength(&mut ui), start, "a tap changes nothing");
    let labels = shown(&mut ui);
    assert!(labels.iter().any(|l| l == "Strength"), "{labels:?}");
    for other in ["Size", "Softness", "Symmetry", "Undo", "Menu 1"] {
        assert!(
            !labels.iter().any(|l| l == other),
            "not {other}: {labels:?}"
        );
    }
    let knob = knob(&mut ui, "Strength");
    assert!(
        apart(knob, at) <= 1.0,
        "its knob {knob:?} is under the pointer {at:?}"
    );
    let info = ui.ok("ui.quick", json!({}));
    assert_eq!(
        (info["open"].clone(), info["slider"].clone()),
        (json!(true), json!("Strength"))
    );

    // The wheel steps it a hundredth of its range a notch, and the view stays.
    ui.input(&[Input::Wheel { dx: 0.0, dy: 1.0 }], Modifiers::default());
    let stepped = strength(&mut ui);
    assert!(
        (stepped - start - 0.01).abs() < 1e-6,
        "{start} to {stepped}"
    );
    assert!(ui.logged("view.dolly").is_empty(), "the view stays");

    ui.tap("Escape", Modifiers::default());
    assert!(shown(&mut ui).is_empty(), "Escape closes it");
    assert_eq!(ui.ok("ui.quick", json!({}))["slider"], json!(null));
    // A press outside closes it too; Shift+Space still opens the whole menu.
    ui.tap("S", Modifiers::default());
    assert_eq!(shown(&mut ui).first().map(String::as_str), Some("Size"));
    let [x0, y0, x1, y1] = ui.rect("viewport");
    ui.tap_at(x0 + (x1 - x0) * 0.8, y0 + (y1 - y0) * 0.8);
    assert!(shown(&mut ui).is_empty(), "a press outside closes it");
    ui.tap("Space", crate::rig::held("Shift"));
    assert!(
        shown(&mut ui).iter().any(|l| l == "Symmetry"),
        "the whole menu"
    );
    assert_eq!(ui.ok("ui.quick", json!({}))["slider"], json!(null));
}

/// A key bound to drag a control's argument shows that control's slider alone, by its label;
/// `ui.quick` `slider` shows one by its label and refuses a label no slider has.
#[test]
fn a_control_slider_shows_alone_by_its_key_or_by_name() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let drag = json!({ "mask_opacity": "drag" });
    let bind = json!({ "bind": "K", "command": "view.display", "args": drag });
    ui.ok("ui.keymap", bind);
    pointer(&mut ui);
    ui.tap("K", Modifiers::default());
    assert_eq!(shown(&mut ui), ["Mask opacity"]);
    let opacity = |ui: &mut Ui| ui.state()["view.mask_opacity"].as_f64().unwrap_or_default();
    let start = opacity(&mut ui);
    ui.input(&[Input::Wheel { dx: 0.0, dy: -1.0 }], Modifiers::default());
    let stepped = opacity(&mut ui);
    assert!(
        (start - stepped - 0.01).abs() < 1e-6,
        "{start} to {stepped}"
    );
    ui.tap("Escape", Modifiers::default());

    let info = ui.ok("ui.quick", json!({ "slider": "Softness" }));
    assert_eq!(
        (info["open"].clone(), info["slider"].clone()),
        (json!(true), json!("Softness"))
    );
    assert_eq!(shown(&mut ui), ["Softness"]);
    let (code, message) = ui.err("ui.quick", json!({ "slider": "Loudness" }));
    assert_eq!(code, gs_protocol::ErrorCode::NotFound, "{message}");
    let both = json!({ "slider": "Size", "open": false });
    assert_eq!(
        ui.err("ui.quick", both).0,
        gs_protocol::ErrorCode::ArgsInvalid
    );
}
