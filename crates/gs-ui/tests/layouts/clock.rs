//! The window's clock (`Studio::at`): a modifier key's tap or hold and a slider key's
//! activation time are measured in seconds by the window's clock, however far apart its frames
//! are (a pen moving fast draws one every few milliseconds, a still one none), and a slider key
//! held past its time shows the value it sets with no input after the press. Tests move the
//! clock with [`Ui::wait`]; nothing sleeps.

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::rig::{Ui, held, key};

/// `n` frames `every` seconds apart, with no input: the window drawing quickly.
fn frames(ui: &mut Ui, n: usize, every: f64) {
    for _ in 0..n {
        ui.wait(every);
        ui.settle();
    }
}

/// Moves the window's clock on `secs` with no input, and says whether the interface asked to
/// draw by then. Draws nothing, so once it asks, it stays asking.
fn asks_to_draw(ui: &mut Ui, secs: f64) -> bool {
    let mut studio = ui.host.lock();
    let now = studio.now_ns() + (secs * 1e9) as u64;
    studio.at(now);
    studio.dirty
}

#[test]
fn a_modifier_tap_and_hold_are_measured_in_seconds_not_frames() {
    let mut ui = Ui::new();
    ui.ok("ui.settings", json!({ "sticky_modifiers": true }));
    let slot = |ui: &mut Ui| ui.state()["brush.slot.held"].clone();
    // Half a second in two frames is a long hold: momentary.
    ui.input(&[], held("Ctrl"));
    ui.wait(0.5);
    ui.input(&[], Modifiers::default());
    assert_eq!(slot(&mut ui), Value::Null, "half a second is a hold");
    // A tenth of a second over 30 quick frames is a tap: Ctrl stays on.
    ui.input(&[], held("Ctrl"));
    frames(&mut ui, 30, 0.004);
    ui.input(&[], Modifiers::default());
    assert_eq!(slot(&mut ui), "mask", "a tenth of a second is a tap");
}

#[test]
fn a_slider_key_waits_its_time_in_seconds_not_frames() {
    let mut ui = Ui::new();
    let none = Modifiers::default();
    let at = |x: f64| Input::MouseMove { x, y: 300.0 };
    let size = |ui: &mut Ui| ui.state()["brush.size_px"].clone();
    let quick_open = |ui: &mut Ui| !ui.widgets("quick").is_empty();
    ui.input(&[at(400.0)], none);
    // Let go after a tenth of a second over 30 quick frames, under the quarter second: a tap.
    ui.input(&[key("S", true, none)], none);
    frames(&mut ui, 30, 0.004);
    ui.input(&[key("S", false, none)], none);
    assert!(quick_open(&mut ui), "a tap shows the slider");
    assert_eq!(size(&mut ui), json!(64.0), "and sets nothing");
    ui.tap("Escape", none);
    assert!(!quick_open(&mut ui));
    // Held three tenths of a second with no frame between: past the time, the move sets it.
    ui.input(&[key("S", true, none)], none);
    ui.wait(0.3);
    ui.input(&[at(480.0)], none);
    ui.input(&[key("S", false, none)], none);
    assert_eq!(size(&mut ui), json!(144.0), "64 and 80");
    assert!(!quick_open(&mut ui), "a hold shows no slider");
}

#[test]
fn a_slider_key_held_past_its_time_shows_its_value_with_no_input() {
    let mut ui = Ui::new();
    let none = Modifiers::default();
    let [x, y] = ui.middle();
    ui.pen(x, y);
    ui.input(&[key("S", true, none)], none);
    // The frames egui draws after an event, then nothing until the quarter second.
    ui.wait(0.1);
    assert!(
        !asks_to_draw(&mut ui, 0.1),
        "nothing to draw before the time"
    );
    assert!(
        asks_to_draw(&mut ui, 0.1),
        "the time running out draws a frame, with no input"
    );
    ui.settle();
    let labels = ui.labels("viewport");
    assert!(
        labels.iter().any(|l| l == "Size 64 px"),
        "the cursor shows the value the key sets: {labels:?}"
    );
    ui.input(&[key("S", false, none)], none);
    let labels = ui.labels("viewport");
    assert!(
        !labels.iter().any(|l| l == "Size 64 px"),
        "let go, it shows no more: {labels:?}"
    );
}
