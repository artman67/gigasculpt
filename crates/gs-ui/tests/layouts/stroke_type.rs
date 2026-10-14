//! Brush tray > Stroke (decision 0060, section 6; catalog `stroke.type-selector`, decision 0051,
//! section 7): `[Dots|Rect|Free|Slide]` (amended for UI-finish), a button per stroke type on one
//! row, lit for the brush's, each running one `brush.set`; then Spacing, Stabilize and, only
//! while Stabilize is above 0, Steady. The index slider is gone. A type the brush's kernel cannot
//! run is drawn disabled, the engine's reason on hover.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::rig::{Ui, center};

/// The stroke types, by the short label the row draws and by `brush.set` name, in the row's
/// order.
const TYPES: [(&str, &str); 4] = [
    ("Dots", "dots"),
    ("Rect", "drag_rect"),
    ("Free", "freehand"),
    ("Slide", "drag_dot"),
];

/// The Workbench, stroking with `brush`. Drawn before the brush is set: after a call the engine
/// dry-runs the controls of the frame drawn last, so the stroke buttons must have been drawn.
fn workbench(brush: &str) -> Ui {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    ui.settle();
    ui.ok("brush.set", json!({ "brush": brush }));
    ui
}

/// The Brush tray's stroke buttons, in the order drawn.
fn buttons(ui: &mut Ui) -> Vec<Value> {
    let titles = TYPES.map(|t| t.0);
    (ui.widgets("brush").into_iter())
        .filter(|w| w["command"]["args"].get("stroke").is_some())
        .filter(|w| titles.contains(&w["label"].as_str().unwrap_or_default()))
        .collect()
}

/// Which buttons are lit, by place.
fn lit(ui: &mut Ui) -> Vec<usize> {
    let shown = buttons(ui);
    (0..shown.len())
        .filter(|&i| shown[i]["selected"] == true)
        .collect()
}

#[test]
fn each_stroke_button_on_one_row_runs_brush_set_and_lights_alone() {
    let mut ui = workbench("Clay");
    assert_eq!(lit(&mut ui), [0], "Clay strokes with Dots");
    let shown = buttons(&mut ui);
    let titles: Vec<&str> = shown.iter().filter_map(|w| w["label"].as_str()).collect();
    assert_eq!(titles, TYPES.map(|t| t.0), "a button per type, in order");
    let tops: Vec<f64> = (shown.iter())
        .map(|w| w["rect"][1].as_f64().unwrap_or_default())
        .collect();
    assert!(
        tops.iter().all(|&t| (t - tops[0]).abs() < 1.0),
        "one row: {tops:?}"
    );
    for (i, (title, name)) in TYPES.into_iter().enumerate() {
        let button = &buttons(&mut ui)[i];
        assert_eq!(button["command"]["command"], "brush.set", "{title}");
        let args = &button["command"]["args"];
        assert_eq!(args, &json!({ "stroke": name }), "{title}");
        let [x, y] = center(&button["rect"]);
        ui.tap_at(x, y);
        assert_eq!(ui.state()["brush.stroke"], json!(name), "{title} picked");
        assert_eq!(lit(&mut ui), [i], "{title} lit alone");
    }
    let labels = ui.labels("brush");
    assert!(
        !labels.iter().any(|l| l == "Stroke index"),
        "the index slider goes"
    );
}

/// The row's four short labels read apart (decision 0060, amended for UI-finish): no two alike
/// even with a final "s" dropped, as Dots and Dot were. Each fits inside the tray: at 1920 by
/// 1080 points the four on one row, at 640 by 480, where the tray is about 127 points wide, two
/// to a row.
#[test]
fn the_stroke_rows_short_labels_read_apart_and_fit_the_tray_at_1920_and_640() {
    let mut ui = workbench("Clay");
    for (size, rows) in [([1920, 1080], 1), ([640, 480], 2)] {
        ui.host.lock().resize(size, 1.0);
        ui.settle();
        ui.settle();
        let tray = ui.rect("brush > stroke");
        let shown: Vec<Value> = (ui.widgets("brush").into_iter())
            .filter(|w| w["command"]["args"].get("stroke").is_some())
            .collect();
        let labels: Vec<&str> = shown.iter().filter_map(|w| w["label"].as_str()).collect();
        let stems: BTreeSet<&str> = (labels.iter())
            .map(|l| l.strip_suffix('s').unwrap_or(l))
            .collect();
        assert_eq!(
            (labels.len(), stems.len()),
            (4, 4),
            "{labels:?} at {size:?}"
        );
        let rect = |w: &Value| -> [f64; 4] {
            std::array::from_fn(|i| w["rect"][i].as_f64().unwrap_or_default())
        };
        for r in shown.iter().map(rect) {
            let inside = r[0] >= tray[0] - 0.5 && r[2] <= tray[2] + 0.5;
            assert!(inside, "{labels:?} at {size:?}: {r:?} in {tray:?}");
        }
        let tops: BTreeSet<i64> = shown.iter().map(|w| rect(w)[1].round() as i64).collect();
        assert_eq!(tops.len(), rows, "{labels:?} at {size:?}: rows at {tops:?}");
    }
}

/// Spacing and Stabilize sit in Stroke under the buttons; Steady shows only while Stabilize is
/// above 0, under it.
#[test]
fn spacing_and_stabilize_sit_under_the_buttons_and_steady_shows_while_stabilize_is_on() {
    let mut ui = workbench("Clay");
    let stabilize = |ui: &mut Ui, radius: f64| {
        let settings = json!({ "stroke": { "smoothing": { "radius": radius } } });
        ui.ok("brush.set", json!({ "settings": settings }));
    };
    stabilize(&mut ui, 0.0);
    let section = ui.rect("brush > stroke");
    let below = buttons(&mut ui)[0]["rect"][3].as_f64().unwrap_or_default();
    let mut tops = Vec::new();
    for label in ["Spacing", "Stabilize"] {
        let r = ui.rect(&format!("brush > {label}"));
        assert!(r[1] >= below - 1.0, "{label} under the buttons: {r:?}");
        assert!(
            r[3] <= section[3] + 1.0,
            "{label} in Stroke: {r:?} {section:?}"
        );
        tops.push(r[1]);
    }
    assert!(tops[0] < tops[1], "Spacing, then Stabilize");
    let steady = |ui: &mut Ui| ui.labels("brush").iter().any(|l| l == "Steady");
    assert!(!steady(&mut ui), "no Steady while Stabilize is 0");
    stabilize(&mut ui, 1.0);
    assert!(steady(&mut ui), "Steady while Stabilize is on");
    let [on, under] = [ui.rect("brush > Stabilize"), ui.rect("brush > Steady")];
    assert!(under[1] > on[1], "Steady under Stabilize: {on:?} {under:?}");
}

#[test]
fn a_stroke_type_the_brush_cannot_run_is_drawn_disabled_saying_why() {
    let mut ui = workbench("Move");
    let shown = buttons(&mut ui);
    assert_eq!(shown.len(), 4, "every type shows");
    for w in &shown {
        let why = w["reason"].as_str().unwrap_or_default();
        assert_eq!(w["enabled"], false, "{}: Move keeps its drag", w["label"]);
        assert!(why.contains("move needs drag"), "{}: {why}", w["label"]);
    }
    ui.ok("brush.set", json!({ "brush": "Pull Out" }));
    let runs: Vec<bool> = (buttons(&mut ui).iter())
        .map(|w| w["enabled"] != false)
        .collect();
    assert_eq!(
        runs,
        [true, false, true, false],
        "Pull Out takes Dots and Freehand"
    );
}
