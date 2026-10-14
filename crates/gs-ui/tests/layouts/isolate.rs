//! Isolate's options in the window (decision 0028, section 10): with Dynamic on and Isolate
//! off, the 3D view draws the active part alone while the camera moves and every shown part
//! once it stops, sending nothing to the GPU either way; the status line says while Isolate or
//! Dynamic is on; and a click on empty canvas toggles Isolate while its setting is on, a drag
//! or a dab never. The two settings share a row, and End Isolate shows while a part is alone
//! (decision 0060 as amended for R1).

use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::Ui;
use crate::view_options::row;

/// A sphere in the middle and a small cube beside it, active as the part made last; drawn
/// on lavapipe with `looks`.
fn two(looks: bool) -> Ui {
    let mut ui = if looks { Ui::with_looks() } else { Ui::new() };
    ui.ok("primitive.sphere", json!({}));
    ui.ok(
        "primitive.cube",
        json!({ "size": 0.5, "center": [1.5, 0, 0] }),
    );
    ui
}

/// The parts the 3D view draws, as `ui.describe` lists them.
fn drawn(ui: &mut Ui) -> Vec<String> {
    let d = ui.describe();
    serde_json::from_value(d["viewport"]["parts"].clone()).unwrap_or_default()
}

/// The parts `part.list` says show, in list order.
fn shown(ui: &mut Ui) -> Vec<String> {
    let list = ui.ok("part.list", json!({}))["parts"].clone();
    let list = list.as_array().cloned().unwrap_or_default();
    let shown = list.into_iter().filter(|p| p["shown"] == json!(true));
    shown
        .filter_map(|p| Some(p["name"].as_str()?.into()))
        .collect()
}

/// Bytes the window sent to the GPU after the last call.
fn uploaded(ui: &Ui) -> u64 {
    ui.host
        .lock()
        .render
        .as_ref()
        .map_or(0, gs_ui::Render::uploaded)
}

/// The pen at (`x`, `y`) with barrel button 1 held while `down`, as one window frame sends
/// it, and the frame: a barrel drag turns the view.
fn barrel(ui: &mut Ui, [x, y]: [f64; 2], down: bool) {
    let sample = json!({ "samples": [{ "x": x, "y": y, "buttons": u8::from(down) }] });
    ui.ok("pen.input", sample);
    ui.settle();
}

/// The status line's words about Isolate.
fn marks(ui: &mut Ui) -> Vec<String> {
    let labels = ui.labels("status").into_iter();
    labels.filter(|l| l.contains("isolate")).collect()
}

#[test]
fn with_dynamic_on_the_other_parts_hide_only_while_the_camera_moves() {
    let mut ui = two(true);
    let both = ["sphere", "cube"];
    let [x, y] = ui.middle();
    // The barrel down and the view turning: the drag has not ended.
    let turning = |ui: &mut Ui| {
        barrel(ui, [x, y], false);
        for dx in [0.0, 30.0, 60.0] {
            barrel(ui, [x + dx, y], true);
        }
    };
    turning(&mut ui);
    assert_eq!(drawn(&mut ui), both, "Dynamic off: a drag hides nothing");
    barrel(&mut ui, [x + 60.0, y], false);
    let on = ui.ok("ui.settings", json!({ "dynamic_isolate": true }));
    assert_eq!(on["dynamic_isolate"], json!(true));
    assert_eq!(ui.state()["settings.dynamic_isolate"], json!(true));
    turning(&mut ui);
    assert_eq!(uploaded(&ui), 0, "hiding sends nothing to the GPU");
    assert_eq!(
        drawn(&mut ui),
        ["cube"],
        "the active part alone while the view turns"
    );
    assert_eq!(
        shown(&mut ui),
        both,
        "what shows is unchanged: only the drawing hides"
    );
    barrel(&mut ui, [x + 60.0, y], false);
    assert_eq!(uploaded(&ui), 0, "and showing them again sends nothing");
    assert_eq!(drawn(&mut ui), both, "every part again once it stops");
    // A script moving `main` in a gesture hides them the same way; a lone step does not.
    let turn = |end: bool| json!({ "view": "main", "right_deg": 10, "gesture": 7, "end": end });
    ui.ok("view.orbit", turn(false));
    assert_eq!(drawn(&mut ui), ["cube"], "a script's gesture on main");
    ui.ok("view.orbit", turn(true));
    assert_eq!(drawn(&mut ui), both, "its end draws them all");
    ui.ok("view.orbit", json!({ "view": "main", "right_deg": 10 }));
    assert_eq!(drawn(&mut ui), both, "a lone step is not motion");
    // With Isolate on, Dynamic changes nothing: Isolate's parts stay while the view turns.
    ui.ok("part.isolate", json!({ "parts": ["sphere", "cube"] }));
    ui.ok("view.orbit", turn(false));
    assert_eq!(
        drawn(&mut ui),
        both,
        "Isolate on: Dynamic hides nothing more"
    );
    ui.ok("view.orbit", turn(true));
}

#[test]
fn a_window_with_no_renderer_describes_the_parts_dynamic_isolate_draws() {
    // `ui.describe` lists the parts the 3D view draws, with GPU copies or none.
    let mut ui = two(false);
    let both = ["sphere", "cube"];
    ui.ok("ui.settings", json!({ "dynamic_isolate": true }));
    let turn = |end: bool| json!({ "view": "main", "right_deg": 10, "gesture": 7, "end": end });
    ui.ok("view.orbit", turn(false));
    assert_eq!(
        drawn(&mut ui),
        ["cube"],
        "the active part alone while main turns"
    );
    assert_eq!(shown(&mut ui), both, "though both still show");
    ui.ok("view.orbit", turn(true));
    assert_eq!(drawn(&mut ui), both, "every shown part once it stops");
}

#[test]
fn the_status_line_says_while_parts_are_isolated_or_dynamic_isolate_is_on() {
    let mut ui = two(false);
    assert!(marks(&mut ui).is_empty(), "neither: no word");
    ui.hold("header > Isolate");
    let options = ui.labels("isolate");
    for setting in ["Dynamic", "Click on canvas"] {
        assert!(
            options.iter().any(|l| l == setting),
            "{setting} in {options:?}"
        );
    }
    let [x, y] = middle(ui.rect("isolate > Dynamic"));
    ui.tap_at(x, y);
    assert_eq!(ui.state()["settings.dynamic_isolate"], json!(true));
    assert_eq!(marks(&mut ui), ["Dynamic isolate"]);
    ui.ok("part.isolate", json!({ "parts": ["@active"] }));
    assert_eq!(
        marks(&mut ui),
        ["Parts isolated"],
        "Isolate on says so alone"
    );
    ui.ok("part.isolate", json!({}));
    ui.ok("ui.settings", json!({ "dynamic_isolate": false }));
    assert!(marks(&mut ui).is_empty(), "both off: no word");
}

#[test]
fn isolates_settings_share_a_row_and_end_isolate_shows_while_a_part_is_alone() {
    let mut ui = two(false);
    let ends = |ui: &mut Ui| ui.labels("isolate").iter().any(|l| l == "End Isolate");
    ui.hold("header > Isolate");
    assert_eq!(
        row(&mut ui, "isolate", "Dynamic"),
        ["Dynamic", "Click on canvas"]
    );
    assert!(!ends(&mut ui), "nothing alone, nothing to end");
    ui.ok("part.isolate", json!({ "parts": ["@active"] }));
    ui.settle();
    assert!(ends(&mut ui), "a part alone can be ended");
    ui.press("isolate > End Isolate");
    assert_eq!(ui.state()["parts.isolated"], json!(false));
    assert!(!ends(&mut ui), "ended, it goes");
}

#[test]
fn a_click_on_empty_canvas_toggles_isolate_only_while_its_setting_is_on() {
    let mut ui = two(false);
    let view = ui.rect("viewport");
    let [x, y] = [view[0] + 30.0, view[3] - 30.0];
    let isolated = |ui: &mut Ui| ui.state()["parts.isolated"].clone();
    ui.tap_at(x, y);
    assert_eq!(
        isolated(&mut ui),
        json!(false),
        "off at first: a click does nothing"
    );
    ui.ok("ui.settings", json!({ "click_isolate": true }));
    assert_eq!(ui.state()["settings.click_isolate"], json!(true));
    ui.tap_at(x, y);
    assert_eq!(
        isolated(&mut ui),
        json!(true),
        "on: a click isolates the active part"
    );
    assert_eq!(shown(&mut ui), ["cube"]);
    ui.tap_at(x, y);
    assert_eq!(isolated(&mut ui), json!(false), "and the next ends it");
    let roles: Vec<Value> = (ui.logged("part.isolate").into_iter())
        .map(|e| e["role"].clone())
        .collect();
    assert_eq!(
        roles,
        [json!("human"), json!("human")],
        "each click is a logged part.isolate, as a script would send it"
    );
    // A drag on empty canvas turns the view; a tap on the active part is a dab.
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x + 60.0, y, true);
    ui.touch(x + 60.0, y, false);
    assert_eq!(isolated(&mut ui), json!(false), "a drag is no click");
    ui.ok("part.select", json!({ "part": "sphere" }));
    let [mx, my] = ui.middle();
    ui.tap_at(mx, my);
    assert_eq!(
        isolated(&mut ui),
        json!(false),
        "a tap on the model sculpts"
    );
}
