//! Brush tray > Tip's Exact, and its Edit shape popover's profile picker and Curve By Pen
//! (decision 0059 section 4, as amended 2026-10-12; decision 0060 section 6; catalog
//! `brush-settings.curve`): each runs `brush.set` once with its
//! own path, a pick dropping a custom curve's points; they light from the pen's brush's settings
//! in effect and relight on a brush switch, a custom curve lighting no profile; and the Zero Curve
//! and Pen Curve graphs show while Curve By Pen is on, an edit in each reaching `tip.zero_curve`
//! or `tip.pen_curve` as one step.

use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::Ui;

/// The picker's buttons and the profile each picks, from the fullest to the sharpest.
const PROFILES: [(&str, &str); 9] = [
    ("Constant", "constant"),
    ("Hard", "hard"),
    ("Sphere", "sphere"),
    ("Flat top", "flat_top"),
    ("Smooth", "smooth"),
    ("Soft", "soft"),
    ("Linear", "linear"),
    ("Sharp", "sharp"),
    ("Sharp center", "sharp_center"),
];

/// The two switches: Exact in the tray, Curve By Pen in the Edit shape popover.
const SWITCHES: [&str; 2] = ["Exact", "Curve By Pen"];

/// The Workbench with the pen on Draw and Brush tray > Tip opened by a tap on its title row,
/// then its Edit shape popover.
fn curve_section() -> Ui {
    let mut ui = Ui::new();
    ui.ok("brush.set", json!({ "brush": "Draw" }));
    let title = ui.rect("brush > curve");
    ui.tap_at(middle(title)[0], title[1] + 10.0);
    ui.press("brush > Edit shape…");
    ui
}

/// Opens the Edit shape popover again where a press outside it (Exact's) shut it.
fn reopen(ui: &mut Ui) {
    if !ui.labels("brush").iter().any(|l| l == "Falloff curve") {
        ui.press("brush > Edit shape…");
    }
}

/// What `ui.describe` says of the brush tray's widget `label`, scrolled to; null where it does
/// not show.
fn seen(ui: &mut Ui, label: &str) -> Value {
    reopen(ui);
    ui.scroll_to("brush", label);
    let all = ui.widgets("brush");
    all.into_iter()
        .find(|w| w["label"] == label)
        .unwrap_or_default()
}

/// This session's change to the pen's brush's tip, as `session.state` says.
fn tip(ui: &mut Ui) -> Value {
    ui.state()["brush.settings"]["tip"].clone()
}

/// The pen's `brush.set` calls in the log so far.
fn logged(ui: &mut Ui) -> usize {
    ui.logged("brush.set").len()
}

/// A tap on the brush tray's `label`, which runs one logged `brush.set`.
fn tap(ui: &mut Ui, label: &str) {
    reopen(ui);
    let before = logged(ui);
    ui.press(&format!("brush > {label}"));
    assert_eq!(logged(ui), before + 1, "{label}: one logged step");
}

#[test]
fn each_profile_pick_and_curve_switch_runs_brush_set_once_with_its_path() {
    let mut ui = curve_section();
    // From a custom curve: a pick drops its points, which only a custom profile may hold.
    tap(&mut ui, "Invert Falloff curve");
    assert_eq!(tip(&mut ui)["profile"], "custom");
    for (label, name) in PROFILES {
        tap(&mut ui, label);
        let set = tip(&mut ui);
        assert_eq!(set["profile"], name, "{label}: {set}");
        assert_eq!(
            set["points"],
            Value::Null,
            "{label} drops the points: {set}"
        );
    }
    for (label, key) in SWITCHES.into_iter().zip(["accurate_curve", "curve_by_pen"]) {
        for on in [true, false] {
            tap(&mut ui, label);
            assert_eq!(tip(&mut ui)[key], on, "{label} sets tip.{key}");
        }
    }
}

#[test]
fn the_picker_and_switches_light_from_the_brush_in_effect_and_relight_on_a_brush_switch() {
    let mut ui = curve_section();
    let lit = |ui: &mut Ui, profile: &str, on: bool| {
        for (label, name) in PROFILES {
            let selected = seen(ui, label)["selected"] == true;
            assert_eq!(selected, name == profile, "{label} with {profile}");
        }
        for label in SWITCHES {
            assert_eq!(seen(ui, label)["selected"] == true, on, "{label}");
        }
    };
    // Draw's own, with no change made.
    assert_eq!(ui.state()["brush.settings"], json!({}), "no change made");
    lit(&mut ui, "smooth", false);
    // Clay, set by an agent, then Draw's own again and Clay's again.
    let clay = json!({ "profile": "sharp_center", "accurate_curve": true, "curve_by_pen": true });
    ui.ok(
        "brush.set",
        json!({ "brush": "Clay", "settings": { "tip": clay } }),
    );
    lit(&mut ui, "sharp_center", true);
    ui.ok("brush.set", json!({ "brush": "Draw" }));
    lit(&mut ui, "smooth", false);
    ui.ok("brush.set", json!({ "brush": "Clay" }));
    lit(&mut ui, "sharp_center", true);
    // A custom curve lights none, and the heading says what it is.
    let custom = json!({ "profile": "custom", "points": [[0, 1], [0.4, 0.2], [1, 0]] });
    ui.ok("brush.set", json!({ "settings": { "tip": custom } }));
    lit(&mut ui, "custom", true);
    assert!(seen(&mut ui, "Profile: custom curve").is_object());
}

#[test]
fn the_zero_and_pen_curves_show_with_curve_by_pen_and_an_edit_reaches_each_as_one_step() {
    let mut ui = curve_section();
    let graphs = [("Zero Curve", "zero_curve"), ("Pen Curve", "pen_curve")];
    for (graph, _) in graphs {
        assert!(seen(&mut ui, graph).is_null(), "no {graph} while off");
    }
    tap(&mut ui, "Curve By Pen");
    for (graph, key) in graphs {
        // A tap halfway across the graph adds a point there, between its two ends.
        let [x, y] = middle(ui.rect(&format!("brush > {graph}")));
        let before = logged(&mut ui);
        ui.tap_at(x, y);
        assert_eq!(logged(&mut ui), before + 1, "{graph}: one logged step");
        let set = tip(&mut ui)[key]["points"].clone();
        let across: Vec<f64> = (set.as_array().into_iter().flatten())
            .map(|p| p[0].as_f64().unwrap_or(f64::NAN))
            .collect();
        assert_eq!(across.len(), 3, "{graph}: {set}");
        assert!((across[1] - 0.5).abs() < 0.05, "{graph}: {set}");
    }
}
