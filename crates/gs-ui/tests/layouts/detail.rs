//! Scene tray > Detail (decision 0060, section 7, as amended 2026-10-13 for S1): the level
//! stepper reads `n of m` and steps one level each way, gray at its ends with the engine's
//! words; the level's ⋯ lists its picks in order, each gray with the engine's reason while it
//! cannot run, and a pick runs and shuts it; Live's choice, Size and Merge show only while Live
//! is on, and Size's unit follows the choice; Subdivide opens the adjust panel with its smooth
//! UVs.

use serde_json::{Value, json};

use crate::rig::Ui;

/// The level's ⋯ picks, in section 7's order.
const PICKS: [&str; 6] = [
    "Go to level…",
    "Delete lower",
    "Delete higher",
    "Freeze",
    "Unfreeze",
    "Reconstruct lower",
];

/// A small sphere named ball with `levels` detail levels, the top one active, and the scene
/// tray showing Detail alone (`tray_fit.rs` fills the trays with it too).
pub(crate) fn ball(levels: u32) -> Ui {
    let mut ui = Ui::new();
    ui.ok(
        "primitive.sphere",
        json!({ "name": "ball", "subdivisions": 2 }),
    );
    for _ in 1..levels {
        ui.ok("level.subdivide", json!({}));
    }
    ui.open_only(&["detail"]);
    ui
}

/// Draws the tray, then makes a call so the engine says which of the controls drawn cannot
/// run, and draws it again.
fn judged(ui: &mut Ui) {
    ui.settle();
    ui.state();
    ui.settle();
}

/// The scene tray's widget labelled `label`.
fn widget(ui: &mut Ui, label: &str) -> Value {
    crate::pen::widget(ui, "tray", label)
}

/// Why the scene tray's widget `label` is gray; `None` while it runs.
fn gray(ui: &mut Ui, label: &str) -> Option<String> {
    let w = widget(ui, label);
    (w["enabled"] == json!(false)).then(|| w["reason"].as_str().unwrap_or_default().into())
}

/// Whether the scene tray shows a widget labelled `label`.
fn shows(ui: &mut Ui, label: &str) -> bool {
    ui.labels("tray").iter().any(|l| l == label)
}

/// The level's ⋯ picks the scene tray shows, in its order.
fn picks(ui: &mut Ui) -> Vec<String> {
    let labels = ui.labels("tray").into_iter();
    labels.filter(|l| PICKS.contains(&l.as_str())).collect()
}

/// The Detail size sliders the scene tray shows.
fn sizes(ui: &mut Ui) -> Vec<Value> {
    let widgets = ui.widgets("tray").into_iter();
    let size = |w: &Value| {
        w["label"]
            .as_str()
            .is_some_and(|l| l.starts_with("Detail size"))
    };
    widgets
        .filter(|w| w["role"] == "slider" && size(w))
        .collect()
}

#[test]
fn the_stepper_steps_one_level_each_way_and_shows_n_of_m() {
    let mut ui = ball(3);
    assert!(shows(&mut ui, "3 of 3"), "{:?}", ui.labels("tray"));
    ui.press("tray > ‹");
    assert_eq!(ui.state()["parts.level"], json!(2), "one level down");
    assert!(shows(&mut ui, "2 of 3"), "{:?}", ui.labels("tray"));
    ui.press("tray > ‹");
    assert!(shows(&mut ui, "1 of 3"), "{:?}", ui.labels("tray"));
    ui.press("tray > ›");
    assert_eq!(ui.state()["parts.level"], json!(2), "one level up");
    assert!(shows(&mut ui, "2 of 3"), "{:?}", ui.labels("tray"));
    assert_eq!(ui.logged("level.step").len(), 3, "each tap one step");
}

/// Where the scene tray's widgets labelled `labels` sit up and down: each one's middle.
fn middles(ui: &mut Ui, labels: &[&str]) -> Vec<f64> {
    let middle = |r: [f64; 4]| f64::midpoint(r[1], r[3]);
    (labels.iter())
        .map(|l| middle(ui.rect(&format!("tray > {l}"))))
        .collect()
}

/// Whether `middles` share one row.
fn one_row(middles: &[f64]) -> bool {
    middles.iter().all(|y| (y - middles[0]).abs() <= 2.0)
}

/// At the tray's default width, with no `Level` label (the heading says Detail): the stepper,
/// Subdivide and the level's ··· share one row, and with Live on Brush, Screen and Units share
/// a row of their own under `Live [On]` (decision 0060 as amended for S1, the two wraps its
/// pictures showed).
#[test]
fn the_stepper_subdivide_and_the_level_menu_share_a_row_and_the_modes_one_under_live() {
    let mut ui = ball(3);
    assert!(!shows(&mut ui, "Level"), "{:?}", ui.labels("tray"));
    let stepper = middles(&mut ui, &["‹", "3 of 3", "›", "Subdivide", "···"]);
    assert!(one_row(&stepper), "one row: {stepper:?}");
    ui.press("tray > On");
    let modes = middles(&mut ui, &["Brush", "Screen", "Units"]);
    assert!(one_row(&modes), "one row: {modes:?}");
    let live = middles(&mut ui, &["Live"]);
    assert!(modes[0] > live[0] + 2.0, "under Live: {modes:?} {live:?}");
}

/// At the top › is gray and at the base ‹ is, each with the words the engine gives for the
/// step that would do nothing; a part with one level has both gray. Frozen levels and an empty
/// scene gray both with the engine's own refusal.
#[test]
fn the_stepper_s_ends_are_gray_with_the_engine_s_words() {
    let mut ui = ball(3);
    let top = "part 'ball': the top is active, there is no level above";
    assert_eq!(gray(&mut ui, "›").as_deref(), Some(top));
    assert_eq!(gray(&mut ui, "‹"), None, "a level below");
    ui.ok("level.set", json!({ "level": 1 }));
    judged(&mut ui);
    let base = "part 'ball': the base is active, there is no level below";
    assert_eq!(gray(&mut ui, "‹").as_deref(), Some(base));
    assert_eq!(gray(&mut ui, "›"), None, "a level above");
    ui.ok("level.delete_higher", json!({}));
    judged(&mut ui);
    assert!(shows(&mut ui, "1 of 1"), "{:?}", ui.labels("tray"));
    for arrow in ["‹", "›"] {
        let why = gray(&mut ui, arrow).unwrap_or_default();
        assert!(
            why.starts_with("part 'ball' has one level: "),
            "{arrow}: {why}"
        );
    }
    // Frozen: the count set aside, and the engine's refusal on both.
    let mut ui = ball(3);
    ui.ok("level.freeze", json!({}));
    judged(&mut ui);
    let labels = ui.labels("tray");
    assert!(labels.iter().any(|l| l.ends_with(" frozen")), "{labels:?}");
    for arrow in ["‹", "›"] {
        let why = gray(&mut ui, arrow).unwrap_or_default();
        assert!(why.contains("frozen detail levels"), "{arrow}: {why}");
    }
    // An empty scene.
    let mut ui = Ui::new();
    ui.open_only(&["detail"]);
    assert!(shows(&mut ui, "none"), "{:?}", ui.labels("tray"));
    for arrow in ["‹", "›"] {
        let why = gray(&mut ui, arrow);
        assert_eq!(why.as_deref(), Some("the scene has no parts"), "{arrow}");
    }
}

#[test]
fn the_level_menu_lists_its_picks_in_order_runs_one_and_shuts() {
    let mut ui = ball(3);
    ui.ok("level.step", json!({ "by": -1 }));
    judged(&mut ui);
    assert_eq!(picks(&mut ui), Vec::<String>::new(), "shut at first");
    ui.press("tray > ···");
    assert_eq!(picks(&mut ui), PICKS, "open, in order");
    ui.press("tray > Delete higher");
    assert_eq!(ui.logged("level.delete_higher").len(), 1, "the pick ran");
    assert_eq!(ui.state()["parts.levels"], json!(2), "level 3 went");
    assert_eq!(picks(&mut ui), Vec::<String>::new(), "a pick shuts it");
}

/// Each pick is gray with the engine's reason while it cannot run: with one level, Freeze, Unfreeze
/// and both deletes; at the top, Delete higher; with the levels frozen, every pick that waits
/// for them. Go to level… and Reconstruct lower are never gray (the engine says no to a
/// rebuild only once it tries).
#[test]
fn a_level_pick_that_cannot_run_is_gray_with_the_reason() {
    let mut ui = ball(1);
    ui.press("tray > ···");
    judged(&mut ui);
    let one = "part 'ball' has one level: there are no levels to freeze";
    assert_eq!(gray(&mut ui, "Freeze").as_deref(), Some(one));
    let thawed = "part 'ball' is not frozen";
    assert_eq!(gray(&mut ui, "Unfreeze").as_deref(), Some(thawed));
    for pick in ["Delete lower", "Delete higher"] {
        let why = gray(&mut ui, pick).unwrap_or_default();
        assert!(
            why.starts_with("part 'ball' has one level: "),
            "{pick}: {why}"
        );
    }
    for pick in ["Go to level…", "Reconstruct lower"] {
        assert_eq!(gray(&mut ui, pick), None, "{pick}");
    }
    ui.ok("level.subdivide", json!({}));
    judged(&mut ui);
    assert_eq!(gray(&mut ui, "Freeze"), None, "with two levels");
    let top = "part 'ball': the top is active, there is no level above";
    assert_eq!(gray(&mut ui, "Delete higher").as_deref(), Some(top));
    assert_eq!(gray(&mut ui, "Delete lower"), None, "a level below");
    ui.ok("level.freeze", json!({}));
    judged(&mut ui);
    assert_eq!(gray(&mut ui, "Unfreeze"), None, "frozen");
    for pick in ["Delete lower", "Delete higher", "Freeze"] {
        let why = gray(&mut ui, pick).unwrap_or_default();
        assert!(why.contains("frozen"), "{pick}: {why}");
    }
}

#[test]
fn live_s_choice_size_and_merge_show_only_while_live_is_on() {
    let mut ui = ball(1);
    let hidden = [
        "Brush",
        "Screen",
        "Units",
        "Merge",
        "Detail size (brush)",
        "Merge length",
    ];
    assert!(
        shows(&mut ui, "Live") && shows(&mut ui, "On"),
        "{:?}",
        ui.labels("tray")
    );
    for label in hidden {
        assert!(!shows(&mut ui, label), "{label} while Live is off");
    }
    ui.press("tray > On");
    assert_eq!(ui.state()["live_detail.on"], json!(true), "the switch");
    for label in hidden {
        assert!(
            shows(&mut ui, label),
            "{label} while on: {:?}",
            ui.labels("tray")
        );
    }
    ui.ok("live_detail.set", json!({ "combined": false }));
    ui.settle();
    assert!(shows(&mut ui, "Merge"), "Merge's switch stays");
    assert!(!shows(&mut ui, "Merge length"), "its length goes with it");
}

/// One Size slider shows at a time, the chosen measure's, setting the mode with the size: a
/// share of the brush, screen pixels, or scene units, which Units asks for.
#[test]
fn size_s_unit_follows_the_choice() {
    let mut ui = ball(1);
    ui.press("tray > On");
    let shown = sizes(&mut ui);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0]["label"], "Detail size (brush)");
    let args = &shown[0]["command"]["args"];
    assert_eq!(args["mode"], "brush");
    let size = args["size"].as_f64().unwrap_or_default();
    assert!(
        (size - 0.25).abs() < 1e-6,
        "a quarter of the radius: {size}"
    );
    ui.press("tray > Screen");
    assert_eq!(ui.state()["live_detail.mode"], "screen");
    let shown = sizes(&mut ui);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0]["label"], "Detail size (pixels)");
    let args = &shown[0]["command"]["args"];
    assert_eq!(
        (&args["mode"], args["size"].as_f64()),
        (&json!("screen"), Some(30.0))
    );
    ui.press("tray > Units");
    assert!(
        crate::header::panel(&mut ui, "palette").is_some(),
        "Units asks for its size"
    );
    ui.ok(
        "live_detail.set",
        json!({ "mode": "constant", "size": 0.02 }),
    );
    ui.settle();
    let shown = sizes(&mut ui);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0]["label"], "Detail size (units)");
    let size = shown[0]["command"]["args"]["size"]
        .as_f64()
        .unwrap_or_default();
    assert!((size - 0.02).abs() < 1e-6, "{size}");
}

/// Subdivide in the tray opens the adjust panel for its step, with Smooth UVs to change; the
/// tray's own Subdivide with smooth UVs is gone.
#[test]
fn subdivide_opens_the_adjust_panel_with_its_smooth_uvs_field() {
    let mut ui = ball(1);
    ui.press("tray > Subdivide");
    assert_eq!(ui.logged("level.subdivide").len(), 1, "the tray ran it");
    let shown = crate::header::panel(&mut ui, "adjust").expect("the panel opens");
    assert_eq!(shown["title"], "Subdivide");
    let labels = ui.labels("adjust");
    let uvs = labels.iter().any(|l| l.eq_ignore_ascii_case("smooth uvs"));
    assert!(uvs, "{labels:?}");
    assert!(!shows(&mut ui, "Subdivide with smooth UVs"));
}
