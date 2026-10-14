//! The scene tray's Live Detail switch under a brush whose automask keeps Live Detail off
//! (catalog `brush-settings.auto-masking`; decision 0021, section 3, amended twice on
//! 2026-10-12): it is dimmed, `live_detail.why_not` on hover, read from `session.state`'s
//! `live_detail.blocked` as an agent reads it, and while Live Detail is on the status line names
//! the automask. Dimmed, it still shows whether Live Detail is on and still turns it on and off,
//! as Backslash does (decision 0025, section 6, amended 2026-10-12).

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::rig::Ui;

/// What the status line says while Live Detail is on under a backface brush.
const LINE: &str = "No Live Detail: backface automask";

/// The scene tray's Detail control `label` (the first, Live's switch, for `On`), as
/// `ui.describe` lists it.
fn control(ui: &mut Ui, label: &str) -> Value {
    ui.open_only(&["detail"]);
    let found = (ui.widgets("tray").into_iter()).find(|w| w["label"] == label);
    found.unwrap_or_else(|| panic!("the scene tray shows {label}"))
}

/// Whether the status line shows `part`.
fn says(ui: &mut Ui, part: &str) -> bool {
    ui.settle();
    ui.labels("status").iter().any(|l| l == part)
}

/// `brush.set`'s arguments turning the brush's backface automask on (1) or off (0).
fn backface(on: f32) -> Value {
    json!({ "settings": { "automask": { "backface": on } } })
}

/// A sphere, sculpted with a backface brush.
fn backface_sphere() -> Ui {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 12 }));
    ui.ok("brush.set", backface(1.0));
    ui
}

/// Whether Live Detail is on.
fn on(ui: &mut Ui) -> Value {
    ui.state()["live_detail.on"].clone()
}

#[test]
fn the_live_detail_switch_dims_under_a_backface_brush_and_the_status_line_says_why() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 12 }));
    let switch = control(&mut ui, "On");
    assert_eq!(switch["reason"], Value::Null, "Draw refines: {switch}");
    ui.ok("brush.set", backface(1.0));
    let switch = control(&mut ui, "On");
    let state = ui.state();
    assert_eq!(state["live_detail.blocked"], "backface");
    assert_eq!(
        switch["reason"], state["live_detail.why_not"],
        "dimmed, why on hover"
    );
    assert_eq!(switch["enabled"], true, "and it still runs: {switch}");
    assert!(!says(&mut ui, LINE), "Live Detail is off");
    ui.ok("live_detail.set", json!({ "on": true }));
    assert!(says(&mut ui, LINE), "{:?}", ui.labels("status"));
    let mode = control(&mut ui, "Brush");
    assert_eq!(
        mode["reason"],
        Value::Null,
        "the settings stay for the next brush"
    );
    let switch = control(&mut ui, "On");
    assert_eq!(
        switch["reason"], state["live_detail.why_not"],
        "on, still dimmed"
    );
    ui.ok("brush.set", backface(0.0));
    let switch = control(&mut ui, "On");
    assert_eq!(switch["reason"], Value::Null, "Draw refines again");
    assert!(!says(&mut ui, LINE));
}

#[test]
fn a_dimmed_switch_still_reads_as_on_or_off() {
    let mut ui = backface_sphere();
    for lit in [true, false] {
        ui.ok("live_detail.set", json!({ "on": lit }));
        let switch = control(&mut ui, "On");
        assert_ne!(switch["reason"], Value::Null, "dimmed: {switch}");
        assert_eq!(switch["role"], "toggle", "{switch}");
        assert_eq!(switch["shows"]["value"], "live_detail.on", "{switch}");
        assert_eq!(switch["selected"], lit, "{switch}");
    }
}

#[test]
fn the_dimmed_switch_and_backslash_both_turn_live_detail_on_and_off() {
    let mut ui = backface_sphere();
    assert_eq!(on(&mut ui), false);
    control(&mut ui, "On");
    ui.press("tray > On");
    assert_eq!(on(&mut ui), true, "a tap on the dimmed switch");
    assert!(says(&mut ui, LINE), "{:?}", ui.labels("status"));
    ui.tap("Backslash", Modifiers::default());
    assert_eq!(on(&mut ui), false, "Backslash, as the switch");
    assert!(!says(&mut ui, LINE));
    ui.tap("Backslash", Modifiers::default());
    assert_eq!(on(&mut ui), true);
    control(&mut ui, "On");
    ui.press("tray > On");
    assert_eq!(on(&mut ui), false, "the switch turns it off too");
}
