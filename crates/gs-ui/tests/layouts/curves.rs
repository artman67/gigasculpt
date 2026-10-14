//! The curve editor's tools and the other pressure curves through the window (catalogs
//! `interface.curve-editor`, `brush-settings.tablet-pressure` and `interface.pen-settings`,
//! decision 0048 and decision 0036 section 6 as amended): Copy and Paste reproduce a curve, in
//! its own graph or another; Invert and the falloff curve's Steps each run `brush.set` once; a
//! tap on a point turns it sharp, and dragged off the graph and back it turns again; the
//! strength, spacing, Brush Modifier and imbed curves switch off and on as size's does, and a
//! point dragged near a pressure graph's top lands on it; the Pen set sets the pen's feel, one
//! logged step a drag, and the eraser end. Each point's reach lies within its graph, and Wrap
//! Mode's slider sets the tip's `wrap`, fits its row and follows a brush switch (decision 0025,
//! rule 12 and section 3, as amended 2026-10-12). The falloff curve's editor opens from Brush
//! tray > Tip (decision 0060, section 6).

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::Ui;

/// A rectangle `ui.describe` reports, in pixels.
fn pixels(rect: &Value) -> [f64; 4] {
    std::array::from_fn(|i| rect[i].as_f64().unwrap_or_default())
}

/// The brush tray's widget labelled `label`.
fn widget(ui: &mut Ui, label: &str) -> Value {
    let found = ui
        .widgets("brush")
        .into_iter()
        .find(|w| w["label"] == label);
    found.unwrap_or_else(|| panic!("the brush tray shows {label}"))
}

/// The pen's brush's settings, as `brush.describe` reports them.
fn brush(ui: &mut Ui) -> Value {
    let name = ui.state()["brush"].clone();
    ui.ok("brush.describe", json!({ "brush": name }))["settings"].clone()
}

/// A curve's points, from its list form or its table form.
fn points(curve: &Value) -> Value {
    curve.get("points").unwrap_or(curve).clone()
}

/// Opens Brush tray > Tip with a tap on its title row.
fn open_tip(ui: &mut Ui) {
    let rect = ui.rect("brush > curve");
    ui.tap_at(middle(rect)[0], rect[1] + 10.0);
}

/// Opens Brush tray > Tip, then its Edit shape popover, which holds the falloff curve.
fn open_curve(ui: &mut Ui) {
    open_tip(ui);
    ui.press("brush > Edit shape…");
}

/// Brush tray > Tip (decision 0060, section 6): a row with the tip's picture and Softness, then
/// Edit shape… and Exact. The picture and Edit shape… each open the shape editor beside the
/// tray, with the profiles, the curve, Wrap Mode and the Pressure shape, which the tray itself
/// no longer holds; Escape shuts it.
#[test]
fn tip_holds_its_picture_softness_edit_shape_and_exact_and_the_editor_opens_beside_it() {
    let mut ui = Ui::new();
    open_tip(&mut ui);
    let labels = ui.labels("brush");
    for shown in ["Tip picture", "Softness", "Edit shape…", "Exact"] {
        assert!(labels.iter().any(|l| l == shown), "{shown}: {labels:?}");
    }
    let editor = [
        "Constant",
        "Falloff curve",
        "Reset curve",
        "Wrap Mode",
        "Pressure shape",
    ];
    for kept in editor.iter().chain(&["Falloff", "Accurate Curve"]) {
        assert!(!labels.iter().any(|l| l == kept), "{kept} not in the tray");
    }
    let [picture, softness] = [ui.rect("brush > Tip picture"), ui.rect("brush > Softness")];
    let then = picture[2] <= softness[0] + 1.0;
    assert!(then, "the picture, then Softness: {picture:?} {softness:?}");
    let row = picture[1] < softness[3] && softness[1] < picture[3];
    assert!(row, "one row: {picture:?} {softness:?}");
    let tray = ui.rect("brush");
    for opener in ["Tip picture", "Edit shape…"] {
        ui.press(&format!("brush > {opener}"));
        let shown = ui.labels("brush");
        for inside in editor {
            assert!(shown.iter().any(|l| l == inside), "{opener}: {inside}");
        }
        let graph = ui.rect("brush > Falloff curve");
        assert!(graph[0] >= tray[2] - 1.0, "beside the tray: {graph:?}");
        ui.tap("Escape", Modifiers::default());
        let shut = !ui.labels("brush").iter().any(|l| l == "Falloff curve");
        assert!(shut, "{opener}: Escape shuts it");
    }
}

/// Opens Basics' Pressure popover, opening Basics first when another section shut it.
fn open_pressure(ui: &mut Ui) {
    if !ui.labels("brush").iter().any(|l| l == "Pressure…") {
        let rect = ui.rect("brush > brush");
        ui.tap_at(middle(rect)[0], rect[1] + 10.0);
    }
    press(ui, "Pressure…");
}

/// A tap on `label` in the brush tray, scrolled down to it first.
fn press(ui: &mut Ui, label: &str) {
    ui.scroll_to("brush", label);
    ui.press(&format!("brush > {label}"));
}

/// The pen pressing at `from`, dragging to `to` and lifting there.
fn drag(ui: &mut Ui, [x, y]: [f64; 2], to: [f64; 2]) {
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(to[0], to[1], true);
    ui.touch(to[0], to[1], false);
}

#[test]
fn copy_and_paste_reproduce_a_curve_in_its_own_graph_or_another() {
    let mut ui = Ui::new();
    // Five points, the middle one sharp, in three steps.
    let drawn = json!({
        "points": [[0, 1], [0.3, 0.9], [0.5, 0.4], [0.8, 0.1], [1, 0]],
        "smooth": true,
        "sharp": [2],
        "steps": 3
    });
    let tip = json!({ "profile": "custom", "points": drawn });
    ui.ok("brush.set", json!({ "settings": { "tip": tip } }));
    open_curve(&mut ui);
    let copied = brush(&mut ui)["tip"]["points"].clone();
    press(&mut ui, "Copy Falloff curve");
    let reset = json!({ "profile": null, "points": null });
    ui.ok("brush.set", json!({ "settings": { "tip": reset } }));
    assert_eq!(brush(&mut ui)["tip"]["profile"], "smooth");
    press(&mut ui, "Paste Falloff curve");
    let pasted = brush(&mut ui)["tip"].clone();
    assert_eq!(pasted["profile"], "custom", "{pasted}");
    assert_eq!(pasted["points"], copied, "the same points, kinds and steps");
    // Into another graph, in the Pressure popover: the strength curve takes it as it was.
    open_pressure(&mut ui);
    press(&mut ui, "Paste Strength pressure curve");
    assert_eq!(brush(&mut ui)["pressure"]["strength"], copied);
}

#[test]
fn invert_and_the_falloff_curves_steps_each_run_brush_set_once() {
    let mut ui = Ui::new();
    open_curve(&mut ui);
    press(&mut ui, "Invert Falloff curve");
    let tip = brush(&mut ui)["tip"].clone();
    assert_eq!(tip["profile"], "custom", "{tip}");
    // The smooth profile, 1 at the center and 0 at the rim, upside down.
    let inverted = points(&tip["points"]);
    let height = |i: usize| inverted[i][1].as_f64().unwrap_or(f64::NAN);
    let last = inverted.as_array().map_or(0, |a| a.len() - 1);
    assert!(height(0) < 0.01 && height(last) > 0.99, "{tip}");
    // Steps, dragged from its left end (off) to its right: set once, when the pen lifts.
    ui.scroll_to("brush", "Steps");
    let r = ui.rect("brush > Steps");
    let y = middle(r)[1];
    ui.touch(r[0] + 2.0, y, false);
    ui.touch(r[0] + 2.0, y, true);
    ui.touch(r[2] - 2.0, y, true);
    let before = brush(&mut ui)["tip"]["points"].clone();
    assert!(before.get("steps").is_none(), "nothing set yet: {before}");
    ui.touch(r[2] - 2.0, y, false);
    let set = brush(&mut ui)["tip"]["points"].clone();
    assert!(set["steps"].as_u64().is_some_and(|s| s > 60), "{set}");
    assert_eq!(points(&set), inverted, "the points kept");
}

#[test]
fn a_tap_on_a_point_turns_it_sharp_and_dragged_off_and_back_it_turns_again() {
    let mut ui = Ui::new();
    let drawn = json!({ "points": [[0, 1], [0.5, 0.5], [1, 0]], "smooth": true });
    let tip = json!({ "profile": "custom", "points": drawn });
    ui.ok("brush.set", json!({ "settings": { "tip": tip } }));
    open_curve(&mut ui);
    let sharp = |ui: &mut Ui| brush(ui)["tip"]["points"]["sharp"].clone();
    ui.scroll_to("brush", "Curve point 2");
    ui.press("brush > Curve point 2");
    assert_eq!(sharp(&mut ui), json!([1]), "a tap turns it sharp");
    // Off the graph and back on before the pen lifts: it turns smooth again and stays.
    let at = middle(ui.rect("brush > Curve point 2"));
    let below = ui.rect("brush > Falloff curve")[3] + 60.0;
    ui.touch(at[0], at[1], false);
    ui.touch(at[0], at[1], true);
    ui.touch(at[0], below, true);
    ui.touch(at[0], at[1], true);
    ui.touch(at[0], at[1], false);
    let points = brush(&mut ui)["tip"]["points"].clone();
    assert_eq!(points["sharp"], Value::Null, "smooth again: {points}");
    assert_eq!(
        points["points"].as_array().map(Vec::len),
        Some(3),
        "kept: {points}"
    );
}

#[test]
fn the_other_pressure_curves_switch_as_sizes_does_and_a_point_near_the_top_lands_on_it() {
    let mut ui = Ui::new();
    press(&mut ui, "Pressure…");
    let (straight, flat) = (
        json!([[0.0, 0.0], [1.0, 1.0]]),
        json!([[0.0, 1.0], [1.0, 1.0]]),
    );
    let curve = |ui: &mut Ui, key: &str| points(&brush(ui)["pressure"][key]);
    // Draw's strength follows the pressure as shipped: its switch turns it flat, full at every
    // pressure, and on again as a straight line.
    assert_eq!(curve(&mut ui, "strength"), straight);
    press(&mut ui, "Strength by pressure");
    assert_eq!(curve(&mut ui, "strength"), flat, "off");
    let shown = |ui: &mut Ui| {
        ui.labels("brush")
            .iter()
            .any(|l| l == "Strength pressure curve")
    };
    assert!(!shown(&mut ui), "no graph while off");
    press(&mut ui, "Strength by pressure");
    assert_eq!(curve(&mut ui, "strength"), straight, "on");
    // Its left end dragged to 2 percent under the top lands on the top (decision 0036, section
    // 6 as amended). The graph's area is inset the pen's reach, 16 points, on every side.
    ui.scroll_to("brush", "Strength pressure point 1");
    let graph = ui.rect("brush > Strength pressure curve");
    let top = graph[1] + 16.0 + 0.02 * (graph[3] - graph[1] - 32.0);
    let end = middle(ui.rect("brush > Strength pressure point 1"));
    drag(&mut ui, end, [end[0], top]);
    assert_eq!(curve(&mut ui, "strength")[0], json!([0.0, 1.0]), "snapped");
    // Spacing, the Brush Modifier and depth (imbed) ship flat and switch on as a straight line.
    for (label, key) in [
        ("Spacing by pressure", "spacing"),
        ("Modifier by pressure", "modifier"),
        ("Depth by pressure", "imbed"),
    ] {
        assert_eq!(curve(&mut ui, key), flat, "{key}");
        press(&mut ui, label);
        assert_eq!(curve(&mut ui, key), straight, "{key}");
        press(&mut ui, label);
    }
}

/// Each point of the falloff curve and the strength pressure curve reports a reach that lies
/// within its graph: a press past the graph does not land on the point.
#[test]
fn a_curve_points_reach_lies_within_its_graph() {
    let mut ui = Ui::new();
    open_curve(&mut ui);
    for (graph, point) in [
        ("Falloff curve", "Curve point"),
        ("Strength pressure curve", "Strength pressure point"),
    ] {
        if point.starts_with("Strength") {
            open_pressure(&mut ui);
        }
        ui.scroll_to("brush", &format!("{point} 1"));
        let widgets = ui.widgets("brush");
        let g = widgets.iter().find(|w| w["label"] == graph);
        let g = g.unwrap_or_else(|| panic!("the brush tray shows the {graph}"));
        assert_ne!(g["cut"], true, "the {graph} shows whole");
        let g = pixels(&g["rect"]);
        let named = |w: &&Value| w["label"].as_str().is_some_and(|l| l.starts_with(point));
        let points: Vec<[f64; 4]> = widgets
            .iter()
            .filter(named)
            .map(|w| pixels(&w["rect"]))
            .collect();
        assert!(points.len() >= 2, "the {graph}'s points");
        for (i, r) in points.iter().enumerate() {
            let within = r[0] >= g[0] && r[1] >= g[1] && r[2] <= g[2] && r[3] <= g[3];
            assert!(
                within,
                "{point} {} {r:?} lies within the {graph} {g:?}",
                i + 1
            );
        }
    }
}

/// The Wrap Mode slider the brush's tip shows, as the value its command would set.
fn wrap_shown(ui: &mut Ui) -> Value {
    widget(ui, "Wrap Mode")["command"]["args"]["settings"]["tip"]["wrap"].clone()
}

/// The pen's brush's Wrap Mode: 1 while its tip leaves it out.
fn wrap(ui: &mut Ui) -> Value {
    brush(ui)["tip"].get("wrap").cloned().unwrap_or(json!(1))
}

/// Wrap Mode's slider in Brush tray > Tip's Edit shape popover: its row fits, a drag sets the tip's `wrap` once
/// with `brush.set`, and it shows each brush's own value after a switch.
#[test]
fn wrap_mode_sets_the_tips_wrap_and_follows_a_brush_switch() {
    let mut ui = Ui::new();
    open_curve(&mut ui);
    let r = ui.rect("brush > Wrap Mode");
    assert_ne!(widget(&mut ui, "Wrap Mode")["cut"], true, "its row fits");
    assert_eq!(wrap_shown(&mut ui), json!(1), "off");
    // Dragged from its left end (off) past its right: set once, to 16, when the pen lifts.
    let (y, before) = (middle(r)[1], ui.logged("brush.set").len());
    drag(&mut ui, [r[0] + 2.0, y], [r[2] - 2.0, y]);
    assert_eq!(wrap(&mut ui), json!(16));
    assert_eq!(ui.logged("brush.set").len(), before + 1, "one logged step");
    assert_eq!(wrap_shown(&mut ui), json!(16));
    // Another brush shows its own, and this one its own again.
    let clay = json!({ "brush": "Clay", "settings": { "tip": { "wrap": 3 } } });
    ui.ok("brush.set", clay);
    assert_eq!(wrap_shown(&mut ui), json!(3), "Clay's");
    ui.ok("brush.set", json!({ "brush": "Draw" }));
    let draw = wrap(&mut ui);
    assert_eq!(wrap_shown(&mut ui), draw, "Draw's");
}

#[test]
fn the_pen_set_sets_the_pens_feel_once_a_drag_and_the_eraser_end() {
    let mut ui = Ui::new();
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    // Size sensitivity dragged from 100 a whole row's width left, past -100, on its rail at the
    // row's left: set when the pen lifts.
    let r = ui.rect("settings > Size sensitivity");
    let (x, y, far) = (r[0] + 2.0, middle(r)[1], r[0] + 2.0 - (r[2] - r[0]));
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(far, y, true);
    assert_eq!(ui.state()["pen.size_sensitivity"], json!(100.0), "not yet");
    ui.touch(far, y, false);
    let set = ui.state()["pen.size_sensitivity"]
        .as_f64()
        .unwrap_or_default();
    assert!(set < -95.0, "{set}");
    assert_eq!(ui.logged("pen.set").len(), 1, "one logged step");
    for label in ["Strength sensitivity", "Color sensitivity", "Lazy pressure"] {
        let slider = ui
            .widgets("settings")
            .into_iter()
            .find(|w| w["label"] == label);
        let command = slider.map(|w| w["command"]["command"].clone());
        assert_eq!(command, Some(json!("pen.set")), "{label}");
    }
    assert_eq!(ui.state()["navigation.eraser"], json!("tip"));
    ui.press("settings > Alt");
    assert_eq!(ui.state()["navigation.eraser"], json!("alt"));
}
