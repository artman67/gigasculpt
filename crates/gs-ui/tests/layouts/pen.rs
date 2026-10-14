//! The pen works the panels the way it works the model: a value slider dragged with the tip
//! shows the value as it goes and sets it once, when the pen lifts; a tray too long for the
//! window scrolls when the pen drags it; a tap on a section's title folds it shut or open; the
//! header's Look options open on a tap, pick with the pen (Lit mode's light too) and shut on a
//! tap elsewhere; the brush's falloff curve takes its points from the pen, and so do the
//! pressure curves, the session's in the settings window's Pen set and the brush's own under its
//! falloff curve, each with a switch that turns it off.

use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, center};

#[test]
fn a_slider_dragged_with_the_pen_sets_its_value_once_when_the_pen_lifts() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let r = ui.rect("brush > Strength");
    let y = middle(r)[1];
    // The strength slider spans 0 to 1; touch near its left end and drag to its right end.
    ui.touch(r[0] + 2.0, y, false);
    ui.touch(r[0] + 2.0, y, true);
    ui.touch(r[2] - 2.0, y, true);
    assert_eq!(
        ui.state()["brush.strength"],
        json!(0.4_f32),
        "nothing set while the pen is down"
    );
    let shown = ui.widget("brush", "Strength")["command"]["args"]["strength"].clone();
    assert!(
        shown.as_f64().unwrap_or_default() > 0.9,
        "the slider shows it as it goes: {shown}"
    );
    ui.touch(r[2] - 2.0, y, false);
    let strength = ui.state()["brush.strength"].as_f64().unwrap_or_default();
    assert!(strength > 0.9, "set when the pen lifted: {strength}");
}

#[test]
fn the_stabilize_slider_sets_the_string_in_brush_radii_once_when_the_pen_lifts() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let r = ui.rect("brush > Stabilize");
    let y = middle(r)[1];
    // It spans 0 to 4 brush radii; drag it from its left end to its right.
    ui.touch(r[0] + 2.0, y, false);
    ui.touch(r[0] + 2.0, y, true);
    ui.touch(r[2] - 2.0, y, true);
    assert_eq!(ui.state()["brush.smoothing"], json!(0.0), "nothing set yet");
    // It runs `brush.set` with the stroke settings' own argument (decision 0036 section 6), the
    // one L turns off and on, so it shows L.
    let slider = ui.widget("brush", "Stabilize");
    let radius = slider["command"]["args"]["settings"]["stroke"]["smoothing"]["radius"].as_f64();
    assert!(radius.is_some_and(|v| v > 3.5), "{slider}");
    assert_eq!(slider["key"], json!("L"), "{slider}");
    ui.touch(r[2] - 2.0, y, false);
    let set = ui.state()["brush.smoothing"].as_f64();
    assert_eq!(set, radius, "set when the pen lifted");
}

/// The tip of the brush the pen strokes with, as `brush.describe` reports it.
fn tip(ui: &mut Ui) -> Value {
    let brush = ui.state()["brush"].clone();
    ui.ok("brush.describe", json!({ "brush": brush }))["settings"]["tip"].clone()
}

/// The points of a custom profile, as `brush.describe` reports them.
fn points(tip: &Value) -> Vec<Value> {
    tip["points"]["points"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn the_shape_editor_edits_the_falloff_curve_with_the_pen() {
    let mut ui = Ui::new();
    assert_eq!(tip(&mut ui)["profile"], "smooth");
    // Tip starts shut; a tap on its title row opens it, and Edit shape… opens the editor.
    let rect = ui.rect("brush > curve");
    ui.tap_at(middle(rect)[0], rect[1] + 10.0);
    ui.press("brush > Edit shape…");
    // Drag the middle point down to the graph's foot: set once, when the pen lifts.
    let foot = ui.rect("brush > Falloff curve")[3] - 1.0;
    let [x, y] = middle(ui.rect("brush > Curve point 3"));
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x, foot, true);
    assert_eq!(
        tip(&mut ui)["profile"],
        "smooth",
        "nothing set while the pen is down"
    );
    ui.touch(x, foot, false);
    let set = tip(&mut ui);
    assert_eq!(set["profile"], "custom", "{set}");
    assert_eq!(points(&set).len(), 5, "{set}");
    assert_eq!(points(&set)[2], json!([0.5, 0.0]), "{set}");
    // A tap away from the points adds one there; dragging it off the graph takes it away.
    let [x1, _] = middle(ui.rect("brush > Curve point 1"));
    let [x2, _] = middle(ui.rect("brush > Curve point 2"));
    ui.tap_at(f64::midpoint(x1, x2), foot);
    let added = points(&tip(&mut ui));
    assert_eq!(added.len(), 6, "{added:?}");
    let x = added[1][0].as_f64().unwrap_or_default();
    assert!((x - 0.125).abs() < 0.01, "added halfway: {added:?}");
    let [x, y] = middle(ui.rect("brush > Curve point 2"));
    let below = foot + 100.0;
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x, below, true);
    ui.touch(x, below, false);
    assert_eq!(points(&tip(&mut ui)).len(), 5);
    // Reset curve: the preset's profile again.
    let [x, y] = middle(ui.rect("brush > Reset curve"));
    ui.tap_at(x, y);
    let reset = tip(&mut ui);
    assert_eq!(
        (&reset["profile"], &reset["points"]),
        (&json!("smooth"), &Value::Null)
    );
}

/// The size curves as `brush.set` reports them: the session's, then the pen's brush's own.
fn size_curves(ui: &mut Ui) -> [Value; 2] {
    let b = ui.ok("brush.set", json!({}));
    let own = b["settings"]["pressure"]["size"].clone();
    [b["global_pressure"]["size"].clone(), own]
}

/// Drags the left end of the curve `graph` (a `panel > name`) to half height, and lets go.
fn lift_left_end(ui: &mut Ui, graph: &str, point: &str) {
    let half = middle(ui.rect(graph))[1];
    let [x, y] = middle(ui.rect(point));
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x, half, true);
    ui.touch(x, half, false);
}

/// The height of the first point of a curve `brush.set` took from the pen.
fn first_height(curve: &Value) -> f64 {
    curve["points"][0][1].as_f64().unwrap_or(-1.0)
}

#[test]
fn the_pressure_curves_switch_on_as_a_straight_line_take_the_pen_and_switch_off() {
    let mut ui = Ui::new();
    let straight = json!([[0, 0], [1, 1]]);
    // The session's curve, in the settings window's Pen set: off at first, with no graph.
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    assert_eq!(size_curves(&mut ui), [Value::Null, Value::Null]);
    let graph = |ui: &mut Ui, panel, name| ui.labels(panel).iter().any(|l| l == name);
    assert!(
        !graph(&mut ui, "settings", "Pressure curve"),
        "no graph while off"
    );
    ui.press("settings > Every brush");
    assert_eq!(size_curves(&mut ui), [straight.clone(), Value::Null]);
    lift_left_end(
        &mut ui,
        "settings > Pressure curve",
        "settings > Pressure point 1",
    );
    let set = size_curves(&mut ui)[0].clone();
    assert!((first_height(&set) - 0.5).abs() < 0.05, "{set}");
    ui.press("settings > Every brush");
    assert_eq!(size_curves(&mut ui), [Value::Null, Value::Null]);
    ui.press("settings > Close");

    // The brush's own, in Basics' Pressure popover, which a press inside keeps open.
    ui.press("brush > Pressure…");
    assert!(
        !graph(&mut ui, "brush", "Brush pressure curve"),
        "no graph while off"
    );
    ui.press("brush > This brush");
    assert_eq!(size_curves(&mut ui), [Value::Null, straight]);
    let (curve, point) = (
        "brush > Brush pressure curve",
        "brush > Brush pressure point 1",
    );
    lift_left_end(&mut ui, curve, point);
    let set = size_curves(&mut ui)[1].clone();
    assert!((first_height(&set) - 0.5).abs() < 0.05, "{set}");
    ui.press("brush > This brush");
    assert_eq!(size_curves(&mut ui), [Value::Null, Value::Null]);
    // A press outside shuts it.
    ui.tap_at(900.0, 600.0);
    assert!(!graph(&mut ui, "brush", "Pressure to size"), "shut");
}

/// Size's and Strength's pen toggles (decision 0060, section 6): lit while pen pressure changes
/// the value, each the same switch as its curve's in the Pressure popover.
#[test]
fn the_pen_toggles_beside_size_and_strength_switch_their_pressure_curves() {
    let mut ui = Ui::new();
    let lit = |ui: &mut Ui, label| ui.widget("brush", label)["selected"] == true;
    let curve =
        |ui: &mut Ui, key: &str| ui.ok("brush.set", json!({}))["settings"]["pressure"][key].clone();
    // Draw ships with strength following the pressure and size not.
    assert!(lit(&mut ui, "Strength by pen") && !lit(&mut ui, "Size by pen"));
    ui.press("brush > Size by pen");
    assert_eq!(curve(&mut ui, "size"), json!([[0, 0], [1, 1]]));
    assert!(lit(&mut ui, "Size by pen"));
    ui.press("brush > Strength by pen");
    assert_eq!(
        curve(&mut ui, "strength"),
        json!([[0, 1], [1, 1]]),
        "flat: off"
    );
    assert!(!lit(&mut ui, "Strength by pen"));
    let size = ui.widget("brush", "Size")["rect"].clone();
    let pen = ui.widget("brush", "Size by pen")["rect"].clone();
    let right = size[2].as_f64() < pen[0].as_f64();
    assert!(
        right && size[3].as_f64() > pen[1].as_f64(),
        "at the slider's right: {size} {pen}"
    );
}

/// The tray's header: the brush's name, which opens the brush picker, its family first, and
/// the brush's ⋯ (decision 0060, section 6).
#[test]
fn the_brushs_name_opens_the_picker_with_its_family_first() {
    let mut ui = Ui::new();
    let header = |ui: &mut Ui| {
        let labels = ui.labels("brush");
        labels.into_iter().take(2).collect::<Vec<_>>()
    };
    assert_eq!(header(&mut ui), ["Draw", "···"]);
    ui.ok("brush.set", json!({ "brush": "Smooth" }));
    ui.press("brush > Smooth");
    let labels = ui.labels("brush");
    let at = |name: &str| labels.iter().position(|l| l == name);
    let (own, other) = (at("Smooth Strong"), at("Clay"));
    assert!(own.is_some() && own < other, "the family first: {labels:?}");
    ui.press("brush > Clay");
    assert_eq!(ui.state()["brush"], "Clay");
    assert_eq!(header(&mut ui), ["Clay", "···"], "a pick shuts it");
}

/// A window too short for the scene tray: the workbench layout at 640 pixels high, eight parts
/// filling the list's rows and Live Detail on, so Detail shows every row.
fn short_window(ui: &mut Ui) {
    ui.host.lock().resize([1800, 640], 1.0);
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    for _ in 0..8 {
        ui.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    }
    ui.ok("live_detail.set", json!({ "on": true }));
}

/// The pen drags the scene tray up 200 pixels from its middle.
fn drag_tray_up(ui: &mut Ui) {
    let [x, y] = middle(ui.rect("tray"));
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    for dy in [20.0, 80.0, 140.0, 200.0] {
        ui.touch(x, y - dy, true);
    }
    ui.touch(x, y - 200.0, false);
}

#[test]
fn a_long_tray_scrolls_when_the_pen_drags_it() {
    let mut ui = Ui::with_looks();
    short_window(&mut ui);
    // The lowest control showing whole before the drag (one cut at the tray's foot would be
    // scrolled into sight first).
    let widgets = ui.widgets("tray");
    let whole = widgets
        .iter()
        .rfind(|w| w["cut"] != true && w["role"] != "scroll bar");
    let last = whole.and_then(|w| w["label"].as_str());
    let last = last.unwrap_or_default().to_string();
    let before = ui.rect(&format!("tray > {last}"))[1];
    drag_tray_up(&mut ui);
    let after = ui.rect(&format!("tray > {last}"))[1];
    assert!(
        after < before - 100.0,
        "{last} moved up with the pen: {before} to {after}"
    );
}

#[test]
fn a_scrolled_tray_reports_only_what_it_shows_where_it_shows_it() {
    let mut ui = Ui::new();
    short_window(&mut ui);
    // The first widget, Parts' title naming the active part, whose row has the same words.
    let first = ui.widgets("tray")[0].clone();
    drag_tray_up(&mut ui);
    let same = |w: &Value| w["label"] == first["label"] && w["role"] == first["role"];
    assert!(
        !ui.widgets("tray").iter().any(same),
        "{} scrolled out of sight and is not listed",
        first["label"]
    );
    let d = ui.describe();
    let tray = (d["panels"].as_array().into_iter().flatten()).find(|p| p["id"] == "tray");
    let tray = tray.unwrap_or_else(|| panic!("no tray"));
    let r = |v: &Value| -> [f64; 4] { std::array::from_fn(|i| v[i].as_f64().unwrap_or_default()) };
    let outer = r(&tray["rect"]);
    let listed = ["widgets", "sections"].map(|k| tray[k].as_array().cloned().unwrap_or_default());
    for item in listed.iter().flatten() {
        let at = r(&item["rect"]);
        assert!(
            at[0] >= outer[0] && at[1] >= outer[1] && at[2] <= outer[2] && at[3] <= outer[3],
            "{} is reported at {at:?}, outside the tray at {outer:?}",
            item["label"]
                .as_str()
                .or(item["id"].as_str())
                .unwrap_or_default()
        );
        assert!(at[2] > at[0] && at[3] > at[1], "{item} shows");
    }
}

/// The section `id` of panel `panel` (a tray's section, or a header button with its options),
/// as `ui.describe` reports it: whether it is folded, and lit.
fn section_of(ui: &mut Ui, panel: &str, id: &str) -> Value {
    let d = ui.describe();
    let found = (d["panels"].as_array().into_iter().flatten()).find(|p| p["id"] == panel);
    let sections = found.and_then(|t| t["sections"].as_array().cloned());
    let found = sections.into_iter().flatten().find(|s| s["id"] == id);
    found.unwrap_or_else(|| panic!("no section {id} in {panel}"))
}

/// The section `id` of the scene tray.
fn section(ui: &mut Ui, id: &str) -> Value {
    section_of(ui, "tray", id)
}

#[test]
fn a_tap_on_a_sections_title_folds_it_shut_and_the_layout_file_keeps_it() {
    let mut ui = Ui::new();
    ui.open_only(&["detail"]);
    let open = section(&mut ui, "detail");
    assert_eq!(
        (&open["title"], &open["folded"]),
        (&json!("Detail"), &json!(false))
    );
    let remesh = |ui: &mut Ui| ui.labels("tray").iter().any(|l| l == "Subdivide");
    assert!(remesh(&mut ui), "open, it shows its controls");
    // The title row is the top of the section.
    let rect = ui.rect("tray > detail");
    let [x, y] = [middle(rect)[0], rect[1] + 10.0];
    ui.tap_at(x, y);
    assert_eq!(section(&mut ui, "detail")["folded"], json!(true));
    assert!(!remesh(&mut ui), "folded, only its title row shows");
    let folded = ui.host.lock().shell.layout.layout.folded.clone();
    assert!(folded.contains(&"detail".to_string()), "{folded:?}");
    ui.tap_at(x, y);
    assert_eq!(section(&mut ui, "detail")["folded"], json!(false));
    assert!(remesh(&mut ui), "open again");
}

#[test]
fn the_look_options_open_on_a_tap_pick_with_the_pen_and_shut_on_a_tap_elsewhere() {
    let mut ui = Ui::with_looks();
    ui.open_look();
    let tap_on = |ui: &mut Ui, label: &str| {
        let [x, y] = middle(ui.rect(&format!("look > {label}")));
        ui.tap_at(x, y);
    };
    tap_on(&mut ui, "MatCap Red Wax");
    tap_on(&mut ui, "Lit");
    let state = ui.state();
    assert_eq!(
        (&state["look"], &state["view.shading"]),
        (&json!("red-wax"), &json!("lit"))
    );
    assert!(
        !ui.widgets("look").is_empty(),
        "the options stay open while picking"
    );
    let rows = ui.ok("ui.palette", json!({ "query": "red wax" }))["rows"].clone();
    assert!(
        rows.to_string().contains("Header > Look"),
        "the palette says where the looks live: {rows}"
    );
    ui.ok("ui.palette", json!({ "open": false }));
    // A tap on the 3D view's foot is a press elsewhere.
    let view = ui.rect("viewport");
    ui.tap_at(middle(view)[0], view[3] - 20.0);
    assert!(ui.widgets("look").is_empty(), "a tap elsewhere shuts them");
}

#[test]
fn the_isolate_options_show_a_part_alone_and_end_isolate() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("primitive.cube", json!({}));
    // With Look's options open, a press on Isolate only shuts them (decision 0060 as amended
    // for H5, item 3).
    ui.open_look();
    ui.hold("header > Isolate");
    assert!(
        ui.widgets("look").is_empty() && ui.widgets("isolate").is_empty(),
        "a press on another button only shuts the open options"
    );
    ui.hold("header > Isolate");
    assert!(
        !ui.widgets("isolate").is_empty(),
        "held, Isolate opens them"
    );
    // What part.list says shows, and which of the options' part buttons are lit.
    let showing = |ui: &mut Ui| {
        let list = ui.ok("part.list", json!({}))["parts"].clone();
        let list = list.as_array().cloned().unwrap_or_default();
        let shown = list.iter().filter(|p| p["shown"] == json!(true));
        let shown: Vec<String> = shown
            .filter_map(|p| Some(p["name"].as_str()?.into()))
            .collect();
        let lit = ui
            .widgets("isolate")
            .into_iter()
            .filter(|w| w["selected"] == json!(true));
        let lit: Vec<String> = lit
            .filter_map(|w| Some(w["label"].as_str()?.into()))
            .collect();
        assert_eq!(lit, shown, "a part's button is lit while it shows");
        shown
    };
    let tap_on = |ui: &mut Ui, label: &str| {
        let [x, y] = middle(ui.rect(&format!("isolate > {label}")));
        ui.tap_at(x, y);
    };
    assert_eq!(showing(&mut ui), ["sphere", "cube"]);
    tap_on(&mut ui, "cube");
    assert_eq!(showing(&mut ui), ["cube"], "the cube alone");
    tap_on(&mut ui, "cube");
    assert_eq!(
        showing(&mut ui),
        ["sphere", "cube"],
        "the part shown alone ends it"
    );
    tap_on(&mut ui, "sphere");
    assert_eq!(showing(&mut ui), ["sphere"]);
    tap_on(&mut ui, "End Isolate");
    assert_eq!(showing(&mut ui), ["sphere", "cube"]);
}

/// Each part's eye in Scene tray > Parts lights from the part's own visibility, which Isolate
/// leaves alone: a part Isolate set aside is visible though not shown, and its eye still turns
/// it off and on, which shows once Isolate ends.
#[test]
fn a_parts_eye_shows_its_own_visibility_through_isolate() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("primitive.cube", json!({}));
    // Whether `part`'s eye is lit, where it is, and whether part.list says the part shows.
    let eye = |ui: &mut Ui, part: &str| {
        ui.settle();
        let eyes = ui
            .widgets("tray")
            .into_iter()
            .filter(|w| w["label"] == "Visible");
        let found = (eyes.into_iter()).find(|w| w["command"]["args"]["parts"] == json!([part]));
        let found = found.unwrap_or_else(|| panic!("no eye for {part} in the scene tray"));
        let list = ui.ok("part.list", json!({}))["parts"].clone();
        let listed = list
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["name"] == part);
        let shown = listed.is_some_and(|p| p["shown"] == json!(true));
        (
            found["selected"] == json!(true),
            center(&found["rect"]),
            shown,
        )
    };
    ui.ok("part.isolate", json!({ "parts": ["cube"] }));
    let (lit, at, shown) = eye(&mut ui, "sphere");
    assert_eq!((lit, shown), (true, false), "set aside: visible, not shown");
    ui.tap_at(at[0], at[1]);
    assert!(!eye(&mut ui, "sphere").0, "a tap on its eye turns it off");
    ui.ok("part.isolate", json!({}));
    let (lit, _, shown) = eye(&mut ui, "sphere");
    assert_eq!((lit, shown), (false, false), "End Isolate leaves it hidden");
    let (lit, _, shown) = eye(&mut ui, "cube");
    assert!(lit && shown, "the cube still shows, its eye lit");
}

/// The Isolate button stays lit while a part is shown alone, with its options shut, as the
/// layout file's `shows` names `parts.isolated`; a refused call leaves the light as it was.
#[test]
fn the_isolate_button_is_lit_while_a_part_is_shown_alone() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("primitive.cube", json!({}));
    let lit = |ui: &mut Ui| {
        ui.settle();
        let isolate = section_of(ui, "header", "isolate");
        assert_eq!(isolate["folded"], json!(true), "its options stay shut");
        isolate["lit"] == json!(true)
    };
    assert!(!lit(&mut ui), "nothing isolated");
    ui.ok("part.isolate", json!({ "parts": ["cube"] }));
    assert!(lit(&mut ui), "the cube alone lights it");
    ui.err("part.isolate", json!({ "parts": ["no such part"] }));
    assert!(lit(&mut ui), "a refused call leaves it lit");
    ui.ok("part.isolate", json!({}));
    assert!(!lit(&mut ui), "End Isolate puts it out");
    ui.err("part.isolate", json!({ "parts": ["no such part"] }));
    assert!(!lit(&mut ui), "a refused call leaves it out");
}

#[test]
fn the_look_options_set_lit_modes_tone_environment_and_key_light() {
    let mut ui = Ui::with_looks();
    ui.open_look();
    let tap_on = |ui: &mut Ui, label: &str| {
        let [x, y] = middle(ui.rect(&format!("look > {label}")));
        ui.tap_at(x, y);
    };
    // The environment and the lights show in the modes they light (decision 0060 as amended
    // for R1). With the MatCaps listed the options are taller than the 3D view, and scroll.
    tap_on(&mut ui, "Lit");
    ui.scroll_to("look", "AgX");
    tap_on(&mut ui, "AgX");
    ui.scroll_to("look", "Soft studio");
    tap_on(&mut ui, "Soft studio");
    ui.scroll_to("look", "Key light");
    tap_on(&mut ui, "Key light");
    // Exposure spans -5 to 5: drag it from its middle past its left end.
    let r = ui.rect("look > Exposure");
    let y = middle(r)[1];
    let x = f64::midpoint(r[0], r[2]);
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(r[0] - 40.0, y, true);
    ui.touch(r[0] - 40.0, y, false);
    let state = ui.state();
    assert_eq!(
        (&state["view.transform"], &state["view.environment"]),
        (&json!("agx"), &json!("soft-studio"))
    );
    assert_eq!(
        state["light.1"]["on"],
        json!(false),
        "the key light goes off"
    );
    let exposure = state["view.exposure"].as_f64().unwrap_or_default();
    assert!(exposure < -4.9, "down to -5 stops: {exposure}");
    tap_on(&mut ui, "Key light");
    assert_eq!(ui.state()["light.1"]["on"], json!(true), "and on again");
}
