//! Brush tray > Depth and the tilt controls in Brush tray > Alpha (decision 0053 section 8, as
//! amended 2026-10-12; decision 0058; decision 0060 section 6; catalog `brush-settings.depth`,
//! `brush-settings.modifiers` and `brush-settings.alpha-and-texture`): each control runs
//! `brush.set` once with its own path and is lit from the pen's brush's settings in effect, a
//! preset's own value with no change made and each brush's own after a switch, the ones under a
//! switch showing only while it is on; Brush Placement's circle follows Imbed; the Depth Curve
//! shows with the Depth Mask and edits `depth.curve`; with every section open the tray scrolls
//! and keeps its width; a gesture brush shows every control, and a press or drag it refuses sets
//! nothing, the status line saying why; and folded, Depth's title says Imbed and Gravity.

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, center, listed};

/// The gap between a panel's controls and its scroll bar (decision 0025, rule 11), in pixels.
const GAP: f64 = 4.0;

/// A rectangle `ui.describe` reports, in pixels.
fn pixels(rect: &Value) -> [f64; 4] {
    std::array::from_fn(|i| rect[i].as_f64().unwrap_or_default())
}

/// The Workbench with the pen on `brush`.
fn workbench(brush: &str) -> Ui {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    ui.ok("brush.set", json!({ "brush": brush }));
    ui
}

/// Opens the brush tray's sections `open` and shuts the rest (see [`Ui::open_only`]). Returns
/// the tray's sections.
fn show(ui: &mut Ui, open: &[&str]) -> Vec<String> {
    ui.open_only_in("brush", open)
}

/// The brush tray's widget labelled `label`, scrolled into sight, if it shows anywhere.
fn found(ui: &mut Ui, label: &str) -> Option<Value> {
    ui.scroll_to("brush", label);
    ui.widgets("brush")
        .into_iter()
        .find(|w| w["label"] == label)
}

/// The brush tray's widget labelled `label`, scrolled into sight.
fn widget(ui: &mut Ui, label: &str) -> Value {
    found(ui, label).unwrap_or_else(|| panic!("the brush tray shows {label}"))
}

/// Whether the brush tray shows a widget labelled `label` anywhere, scrolling to look.
fn shows(ui: &mut Ui, label: &str) -> bool {
    found(ui, label).is_some()
}

/// This session's change to the pen's brush at the dotted `path`, as `session.state` says.
fn changed(ui: &mut Ui, path: &str) -> Value {
    let pointer = format!("/{}", path.replace('.', "/"));
    let settings = ui.state()["brush.settings"].clone();
    settings.pointer(&pointer).cloned().unwrap_or(Value::Null)
}

/// The value the slider `label` shows at the dotted `path` of its `brush.set` settings.
fn slider(ui: &mut Ui, label: &str, path: &str) -> Option<f64> {
    let pointer = format!("/{}", path.replace('.', "/"));
    let w = widget(ui, label);
    w["command"]["args"]["settings"].pointer(&pointer)?.as_f64()
}

/// Whether the button `label` is lit.
fn lit(ui: &mut Ui, label: &str) -> bool {
    widget(ui, label)["selected"] == true
}

/// The brush's `brush.set` calls in the log so far.
fn sets(ui: &mut Ui) -> usize {
    ui.logged("brush.set").len()
}

/// Taps the brush tray's button `label`: one logged `brush.set`, which sets `path` to `want`.
fn tap(ui: &mut Ui, label: &str, path: &str, want: &Value) {
    let before = sets(ui);
    ui.press(&format!("brush > {label}"));
    assert_eq!(sets(ui), before + 1, "{label}: one logged step");
    assert_eq!(&changed(ui, path), want, "{label} sets {path}");
}

/// The Depth switch that sets the dotted `path` (Mask's or Infinite's, both reading `On` after
/// their line's label), scrolled into sight: near Imbed, or with the depth curve open, Gravity.
fn switch(ui: &mut Ui, path: &str) -> Value {
    let pointer = format!("/{}", path.replace('.', "/"));
    let sets = |w: &Value| w["command"]["args"]["settings"].pointer(&pointer).is_some();
    for near in ["Imbed", "Gravity"] {
        ui.scroll_to("brush", near);
        let all = ui.widgets("brush").into_iter();
        if let Some(found) = all.filter(|w| w["label"] == "On").find(sets) {
            return found;
        }
    }
    panic!("the On that sets {path}")
}

/// Taps the Depth switch that sets `path`: one logged `brush.set`, which sets it to `want`.
fn flip(ui: &mut Ui, path: &str, want: &Value) {
    let before = sets(ui);
    let [x, y] = center(&switch(ui, path)["rect"]);
    ui.tap_at(x, y);
    assert_eq!(sets(ui), before + 1, "{path}: one logged step");
    assert_eq!(&changed(ui, path), want, "the switch sets {path}");
}

/// Opens Alpha's ⋯ (Hold turn and Map detail), told from the brush's ⋯ by its hint.
fn more(ui: &mut Ui) {
    ui.scroll_to("brush", "Follow");
    let mut all = ui.widgets("brush").into_iter();
    let hint = "Hold turn and Map detail.";
    let more = all.rfind(|w| w["label"] == "···" && w["hint"] == hint);
    let [x, y] = center(&more.unwrap_or_else(|| panic!("Alpha's ⋯ shows"))["rect"]);
    ui.tap_at(x, y);
}

/// Presses the pen on the slider `label`'s rail, 20 points in from its left end, and drags it
/// its row's width to the right when `up`, else to the left, still held. A drag moves the value
/// from where it is by the rail's width end to end, so this takes it to that end of its rail.
/// Returns where it holds.
fn drag_to(ui: &mut Ui, label: &str, up: bool) -> [f64; 2] {
    let r = ui.rect(&format!("brush > {label}"));
    let [x, y] = [r[0] + 20.0, middle(r)[1]];
    let by = if up { r[2] - r[0] } else { r[0] - r[2] };
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x + by, y, true);
    [x + by, y]
}

/// Drags the brush tray's slider `label` to the high end of its rail when `up`, else to its
/// low end: one logged `brush.set`, when the pen lifts, which sets `path` within a tenth of
/// `[low, high]` of that end.
fn slide(ui: &mut Ui, label: &str, path: &str, [low, high]: [f64; 2], up: bool) {
    let before = sets(ui);
    let [x, y] = drag_to(ui, label, up);
    ui.touch(x, y, false);
    assert_eq!(sets(ui), before + 1, "{label}: one logged step");
    let set = changed(ui, path).as_f64().unwrap_or(f64::NAN);
    let want = if up { high } else { low };
    assert!(
        (set - want).abs() < (high - low) / 10.0,
        "{label} sets {path}: {set}"
    );
}

#[test]
fn every_depth_and_tilt_control_runs_brush_set_once_with_its_path() {
    // Clay Build: a plane brush with an alpha, which takes every one of them.
    let mut ui = workbench("Clay Build");
    // Depth starts shut, last in the tray; a tap on its title row opens it.
    assert!(!shows(&mut ui, "Imbed"), "Depth starts shut");
    let rect = ui.rect("brush > depth");
    ui.tap_at(middle(rect)[0], rect[1] + 10.0);
    // Decision 0060, section 6, item 6: Inner and Outer only while Mask is on, X, Y and Z
    // while Infinite is, and Gravity's directions while it is not 0.
    for label in ["Inner Depth", "Outer Depth", "X", "Down"] {
        assert!(!shows(&mut ui, label), "{label} waits for its switch");
    }
    let on = json!(true);
    flip(&mut ui, "depth.mask", &on);
    flip(&mut ui, "depth.infinite", &on);
    // X, Y and Z each send the whole triple with their own axis turned.
    tap(&mut ui, "X", "depth.axes", &json!([true, false, false]));
    tap(&mut ui, "Z", "depth.axes", &json!([true, false, true]));
    tap(&mut ui, "X", "depth.axes", &json!([false, false, true]));
    slide(&mut ui, "Gravity", "post.gravity", [-1.0, 1.0], true);
    for (label, dir) in [
        ("Up", [0.0, 1.0, 0.0]),
        ("Left", [-1.0, 0.0, 0.0]),
        ("Right", [1.0, 0.0, 0.0]),
        ("Back", [0.0, 0.0, -1.0]),
        ("Front", [0.0, 0.0, 1.0]),
        ("Down", [0.0, -1.0, 0.0]),
    ] {
        tap(&mut ui, label, "post.gravity_dir", &json!(dir));
    }
    // Each away from Clay Build's own value: up from the low end, or down from the high end.
    for (label, path, range, up) in [
        ("Imbed", "depth.imbed", [-1.0, 1.0], true),
        ("Inner Depth", "depth.inner", [-1.0, 0.0], true),
        ("Outer Depth", "depth.outer", [0.0, 1.0], false),
    ] {
        slide(&mut ui, label, path, range, up);
    }
    show(&mut ui, &["alpha"]);
    for (label, path, range, up) in [
        ("Turn", "tip.tilt", [-90.0, 90.0], true),
        ("Follow", "tip.align", [0.0, 100.0], false),
    ] {
        slide(&mut ui, label, path, range, up);
    }
    // Alpha's ⋯: a pick shuts it.
    more(&mut ui);
    tap(&mut ui, "Hold turn", "tip.constant_tilt", &on);
    more(&mut ui);
    slide(
        &mut ui,
        "Map detail",
        "tip.alpha.adaptive",
        [0.0, 1.0],
        false,
    );
}

#[test]
fn the_controls_light_from_the_brush_in_effect_and_relight_on_a_brush_switch() {
    let mut ui = workbench("Clay Build");
    show(&mut ui, &["depth", "alpha"]);
    // Clay Build's own Imbed, with no change made this session; its alpha's map size.
    assert_eq!(ui.state()["brush.settings"], json!({}), "no change made");
    let imbed = slider(&mut ui, "Imbed", "depth.imbed").map(|v| v as f32);
    assert_eq!(imbed, Some(0.1), "Clay Build's own Imbed");
    more(&mut ui);
    let adaptive = slider(&mut ui, "Map detail", "tip.alpha.adaptive");
    assert_eq!(adaptive, Some(1.0));
    ui.tap("Escape", Modifiers::default());
    // Draw, set by an agent: every control shows Draw's.
    let draw = json!({
        "depth": { "mask": true, "inner": -0.5, "outer": 0.25, "infinite": true,
                   "axes": [false, true, false] },
        "post": { "gravity": 0.5, "gravity_dir": [1.0, 0.0, 0.0] },
        "tip": { "tilt": 30.0, "constant_tilt": true, "align": 40.0 }
    });
    ui.ok("brush.set", json!({ "brush": "Draw", "settings": draw }));
    let draws = |ui: &mut Ui| {
        for path in ["depth.mask", "depth.infinite"] {
            assert_eq!(switch(ui, path)["selected"], true, "Draw's {path} is lit");
        }
        for label in ["Y", "Right"] {
            assert!(lit(ui, label), "Draw's {label} is lit");
        }
        for label in ["X", "Z", "Down"] {
            assert!(!lit(ui, label), "Draw's {label} is not");
        }
        for (label, path, want) in [
            ("Imbed", "depth.imbed", 0.0),
            ("Inner Depth", "depth.inner", -0.5),
            ("Outer Depth", "depth.outer", 0.25),
            ("Gravity", "post.gravity", 0.5),
            ("Turn", "tip.tilt", 30.0),
            ("Follow", "tip.align", 40.0),
        ] {
            assert_eq!(slider(ui, label, path), Some(want), "Draw's {label}");
        }
        more(ui);
        assert!(lit(ui, "Hold turn"), "Draw's Hold turn is lit");
        ui.tap("Escape", Modifiers::default());
    };
    draws(&mut ui);
    // Clay Build again: its own, Draw's changes gone from the controls; with Infinite off and
    // Gravity at 0, X, Y, Z and the directions hide.
    ui.ok("brush.set", json!({ "brush": "Clay Build" }));
    for path in ["depth.mask", "depth.infinite"] {
        assert_ne!(
            switch(&mut ui, path)["selected"],
            true,
            "Clay Build's {path}"
        );
    }
    for label in ["Y", "Right", "Down"] {
        assert!(!shows(&mut ui, label), "Clay Build hides {label}");
    }
    more(&mut ui);
    assert!(!lit(&mut ui, "Hold turn"), "Clay Build's Hold turn is off");
    ui.tap("Escape", Modifiers::default());
    let imbed = slider(&mut ui, "Imbed", "depth.imbed").map(|v| v as f32);
    assert_eq!(imbed, Some(0.1));
    assert_eq!(slider(&mut ui, "Turn", "tip.tilt"), Some(0.0));
    // And Draw's once more.
    ui.ok("brush.set", json!({ "brush": "Draw" }));
    draws(&mut ui);
}

#[test]
fn brush_placement_is_described_and_its_circle_follows_imbed() {
    let mut ui = workbench("Clay Build");
    show(&mut ui, &["depth"]);
    let view = pixels(&widget(&mut ui, "Brush Placement")["rect"]);
    assert!(
        view[3] - view[1] >= 60.0,
        "a view tall enough to show the band: {view:?}"
    );
    let circle = |ui: &mut Ui| center(&widget(ui, "Brush Placement circle")["rect"])[1];
    let at = circle(&mut ui);
    ui.ok(
        "brush.set",
        json!({ "settings": { "depth": { "imbed": 0.8 } } }),
    );
    let up = circle(&mut ui);
    assert!(up < at - 1.0, "Imbed up raises the circle: {at} to {up}");
    // It follows an Imbed drag while the pen holds it, before anything is set.
    let before = sets(&mut ui);
    let [x, y] = drag_to(&mut ui, "Imbed", false);
    let held = circle(&mut ui);
    assert_eq!(sets(&mut ui), before, "nothing set while the pen holds it");
    ui.touch(x, y, false);
    assert!(
        held > up + 1.0,
        "held at Imbed's low end it sinks: {up} to {held}"
    );
    // The marks and the reach show with their switches.
    let marks = [
        "Outer Depth mark",
        "Inner Depth mark",
        "Infinite Depth reach",
    ];
    for mark in marks {
        assert!(!shows(&mut ui, mark), "{mark} shows only with its switch");
    }
    let both = json!({ "depth": { "mask": true, "infinite": true } });
    ui.ok("brush.set", json!({ "settings": both }));
    let mark = |ui: &mut Ui, label: &str| center(&widget(ui, label)["rect"])[1];
    let (outer, inner) = (mark(&mut ui, marks[0]), mark(&mut ui, marks[1]));
    assert!(
        outer < inner,
        "Outer Depth above Inner Depth: {outer}, {inner}"
    );
    let reach = pixels(&widget(&mut ui, marks[2])["rect"]);
    let view = pixels(&widget(&mut ui, "Brush Placement")["rect"]);
    assert!(
        reach[1] <= view[1] + 1.0 && reach[3] >= view[3] - 1.0,
        "{reach:?} in {view:?}"
    );
}

#[test]
fn the_depth_curve_shows_with_the_mask_and_an_edit_reaches_depth_curve() {
    let mut ui = workbench("Draw");
    show(&mut ui, &["depth"]);
    assert!(!shows(&mut ui, "Depth Curve"), "not with the mask off");
    flip(&mut ui, "depth.mask", &json!(true));
    // A tap on the graph halfway across, near its foot, adds a point there.
    let graph = ui.rect("brush > Depth Curve");
    let before = sets(&mut ui);
    ui.tap_at(middle(graph)[0], graph[3] - 20.0);
    assert_eq!(sets(&mut ui), before + 1, "one logged step");
    let points = changed(&mut ui, "depth.curve.points");
    let at = points.as_array().cloned().unwrap_or_default();
    let added = at
        .iter()
        .find(|p| (p[0].as_f64().unwrap_or(-1.0) - 0.5).abs() < 0.05);
    let low = added.and_then(|p| p[1].as_f64());
    assert!(
        low.is_some_and(|y| y < 0.2),
        "a point near the foot: {points}"
    );
    // The brush's own curve holds them, to the thousandth the editor sends.
    let thousandths = |p: &Value| -> Vec<[i64; 2]> {
        let at = |v: &Value| (v.as_f64().unwrap_or(f64::NAN) * 1000.0).round() as i64;
        (p.as_array().into_iter().flatten())
            .map(|q| [at(&q[0]), at(&q[1])])
            .collect()
    };
    let described = ui.ok("brush.describe", json!({ "brush": "Draw" }));
    let curve = &described["settings"]["depth"]["curve"];
    assert_eq!(
        thousandths(&curve["points"]),
        thousandths(&points),
        "the brush's curve: {curve}"
    );
    // Invert, under it, turns it over with one logged step.
    let before = sets(&mut ui);
    ui.press("brush > Invert Depth Curve");
    assert_eq!(sets(&mut ui), before + 1, "one logged step");
    let turned = changed(&mut ui, "depth.curve.points");
    let ys = |p: &Value| -> Vec<f64> {
        (p.as_array().into_iter().flatten())
            .map(|q| (q[1].as_f64().unwrap_or_default() * 1000.0).round() / 1000.0)
            .collect()
    };
    let flipped: Vec<f64> = ys(&points).iter().map(|y| 1.0 - y).collect();
    assert_eq!(ys(&turned), flipped, "{turned}");
}

#[test]
fn with_every_section_open_the_brush_tray_scrolls_rather_than_widens() {
    let mut ui = workbench("Clay Build");
    // The Depth Curve, the reach and Gravity's directions show too (Curve By Pen's graphs are in
    // Tip's Edit shape popover, beside the tray).
    let both = json!({ "depth": { "mask": true, "infinite": true }, "post": { "gravity": 0.5 } });
    ui.ok("brush.set", json!({ "settings": both }));
    let panel = |d: &Value| {
        let panels = d["panels"].as_array().cloned().unwrap_or_default();
        let found = panels.into_iter().find(|p| p["id"] == "brush");
        pixels(&found.unwrap_or_else(|| panic!("a brush tray"))["rect"])
    };
    let ids = show(&mut ui, &[]);
    let shut = panel(&ui.describe());
    let every: Vec<&str> = ids.iter().map(String::as_str).collect();
    show(&mut ui, &every);
    // At the top, at Stroke's Stabilize, then at Depth's last control, the foot of the tray.
    for at in ["the top", "Stabilize", "Front"] {
        ui.scroll_to("brush", if at == "the top" { "brush" } else { at });
        let d = ui.describe();
        assert_eq!(panel(&d), shut, "the tray keeps its width ({at})");
        let widgets = listed(&d, "brush", "widgets");
        let bar = widgets.iter().find(|w| w["role"] == "scroll bar");
        let bar = pixels(&bar.unwrap_or_else(|| panic!("the tray scrolls ({at})"))["rect"]);
        for w in widgets.iter().filter(|w| w["role"] != "scroll bar") {
            let r = pixels(&w["rect"]);
            let side = w["cut"] == true && r[1] > shut[1] + 1.0 && r[3] < shut[3] - 1.0;
            assert!(
                r[0] >= shut[0] && r[2] <= bar[0] - GAP && !side,
                "{} {r:?} runs past the tray's edge ({at})",
                w["label"]
            );
        }
    }
    assert!(
        shows(&mut ui, "Depth Curve"),
        "the Depth Curve shows at the foot"
    );
}

/// What the status line says went wrong, cleared.
fn problem(ui: &mut Ui) -> String {
    let problem = ui.host.lock().shell.status.problem.take();
    problem.unwrap_or_default()
}

#[test]
fn a_gesture_brush_refuses_its_switches_and_drags_saying_why() {
    let mut ui = workbench("Clip Curve");
    show(&mut ui, &["depth"]);
    for label in ["Brush Placement", "Imbed", "Gravity"] {
        assert_eq!(widget(&mut ui, label)["enabled"], true, "{label} shows");
    }
    // A switch the gesture refuses shows, and a press of it sets nothing, saying why.
    let refused = |ui: &mut Ui, what: &str, before: usize| {
        assert_eq!(sets(ui), before, "{what}: refused");
        let why = problem(ui);
        assert!(why.contains("gesture"), "{what} says why: {why:?}");
    };
    for path in ["depth.mask", "depth.infinite"] {
        let before = sets(&mut ui);
        let [x, y] = center(&switch(&mut ui, path)["rect"]);
        ui.tap_at(x, y);
        refused(&mut ui, path, before);
        assert_ne!(switch(&mut ui, path)["selected"], true, "{path} stays off");
    }
    show(&mut ui, &["alpha"]);
    assert_eq!(widget(&mut ui, "Follow")["enabled"], true, "Follow shows");
    more(&mut ui);
    let before = sets(&mut ui);
    ui.press("brush > Hold turn");
    refused(&mut ui, "Hold turn", before);
    ui.tap("Escape", Modifiers::default());
    // So does a drag of Turn away from 0.
    let before = sets(&mut ui);
    let [x, y] = drag_to(&mut ui, "Turn", true);
    ui.touch(x, y, false);
    assert_eq!(sets(&mut ui), before, "refused");
    assert_eq!(changed(&mut ui, "tip.tilt"), Value::Null);
    let why = problem(&mut ui);
    assert!(why.contains("gesture"), "{why:?}");
}

/// Decision 0060, section 6: a folded section's title says its values, dim, after the title:
/// Depth's Imbed and Gravity, and Scene tray > Detail's level. Open, the values show under it.
#[test]
fn a_folded_depth_and_detail_say_their_values_in_their_titles() {
    let mut ui = workbench("Clay Build");
    show(&mut ui, &[]);
    let said = |ui: &mut Ui, panel: &str, words: &str| ui.labels(panel).iter().any(|l| l == words);
    let labels = ui.labels("brush");
    assert!(
        said(&mut ui, "brush", "Imbed 0.1 · Gravity off"),
        "{labels:?}"
    );
    let pull = json!({ "post": { "gravity": -0.5 } });
    ui.ok("brush.set", json!({ "settings": pull }));
    assert!(said(&mut ui, "brush", "Imbed 0.1 · Gravity -0.5"));
    show(&mut ui, &["depth"]);
    assert!(!said(&mut ui, "brush", "Imbed 0.1 · Gravity -0.5"), "open");
    ui.ok("primitive.sphere", json!({}));
    let level = |ui: &mut Ui| {
        let labels = ui.labels("tray");
        (labels.iter()).any(|l| l.starts_with("Level ") && l.contains(" of "))
    };
    ui.open_only(&[]);
    assert!(
        level(&mut ui),
        "Detail says its level: {:?}",
        ui.labels("tray")
    );
    ui.open_only(&["detail"]);
    assert!(!level(&mut ui), "open");
}
