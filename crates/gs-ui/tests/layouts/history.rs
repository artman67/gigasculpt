//! The History buttons name the step they would take and count the steps that way, read from
//! `session.state` (`names` and `counts` in the controls file), and are disabled, saying why,
//! while there is no step to take (decision 0025, section 6; decision 0016, sections 16 and 17).
//! Adjust opens the adjust panel again, or says there is nothing to adjust (decision 0060).

use serde_json::{Value, json};

use crate::rig::Ui;

/// The scene tray's History button whose label starts with `word`, as `ui.describe` lists it.
fn button(ui: &mut Ui, word: &str) -> Value {
    ui.open_only(&["history"]);
    let found = (ui.widgets("tray").into_iter())
        .find(|w| w["label"].as_str().is_some_and(|l| l.starts_with(word)));
    found.unwrap_or_else(|| panic!("the scene tray shows {word}"))
}

/// Catalog `canvas.undo-redo`: "the button shows the remaining count", and it is greyed out
/// with nothing to travel to.
#[test]
fn the_history_buttons_name_their_step_count_the_rest_and_are_disabled_with_none() {
    let mut ui = Ui::new();
    for word in ["Undo", "Redo"] {
        let fresh = button(&mut ui, word);
        assert_eq!(
            fresh["label"], word,
            "a fresh scene has no step to name or count"
        );
        assert_eq!(fresh["enabled"], false, "{word} has nothing to travel to");
        let why = fresh["reason"].as_str().unwrap_or_default();
        // In the words the status line would say it (decision 0025, section 6, amended
        // 2026-10-14).
        let nothing = format!("nothing to {}", word.to_lowercase());
        assert!(
            why.to_lowercase().contains(&nothing),
            "{word} says why: {why}"
        );
    }
    ui.ok("primitive.sphere", json!({ "subdivisions": 12 }));
    let stroke = json!({ "brush": "Draw", "size": "20%", "samples": [{ "position": [0, 0, 1] }] });
    ui.ok("brush.stroke", stroke);
    let undo = button(&mut ui, "Undo");
    assert_eq!(undo["label"], "Undo Draw stroke (2)", "two steps to undo");
    assert_ne!(undo["enabled"], false, "there is a stroke to undo");
    assert_eq!(undo["command"]["command"], "history.undo");
    let none = button(&mut ui, "Redo");
    assert_eq!(
        (none["label"].as_str(), none["enabled"].as_bool()),
        (Some("Redo"), Some(false))
    );
    ui.ok("history.undo", json!({}));
    let redo = button(&mut ui, "Redo");
    assert_eq!(redo["label"], "Redo Draw stroke (1)");
    assert_ne!(redo["enabled"], false, "the stroke undone can be redone");
    assert_eq!(button(&mut ui, "Undo")["label"], "Undo Add sphere (1)");
    assert_eq!(
        ui.state()["history.undos"],
        json!(1),
        "scripts read the counts too"
    );
    assert_eq!(ui.state()["history.redos"], json!(1));
}

/// The scene tray `points` wide, as a drag on its inner edge leaves it (`layout.toml` keeps it).
pub fn narrow(ui: &mut Ui, points: f32) {
    let mut studio = ui.host.lock();
    let widths = &mut studio.shell.layout.layout.widths;
    widths.insert("tray".into(), points);
}

/// The width and height of `widget` as `ui.describe` lists it, in pixels.
pub fn size(widget: &Value) -> [f64; 2] {
    let r = |i: usize| widget["rect"][i].as_f64().unwrap_or_default();
    [r(2) - r(0), r(3) - r(1)]
}

/// In a scene tray about 190 points wide, a History button naming a long step keeps one row:
/// its name is cut to the tray's width, and stays whole in `ui.describe`, as on hover (decision
/// 0062, row 794).
#[test]
fn a_history_button_naming_a_long_step_keeps_one_row_in_a_narrow_tray() {
    use gs_protocol::{BatchParams, Outcome, Request, Step};
    let mut ui = Ui::new();
    let sphere = |i: u32| Step {
        command: "primitive.sphere".into(),
        args: json!({ "center": [i, 0, 0], "subdivisions": 1 }),
    };
    let batch = BatchParams {
        commands: (0..3).map(sphere).collect(),
        if_rev: None,
    };
    let made = ui.session.request(Request::Batch(batch));
    assert!(matches!(made.outcome, Outcome::Ok(_)), "{:?}", made.outcome);
    narrow(&mut ui, 190.0);
    let undo = button(&mut ui, "Undo");
    let whole = "Undo Add sphere and 2 more (1)";
    assert_eq!(undo["label"], whole, "named whole");
    let [_, one_row] = size(&button(&mut ui, "Adjust"));
    assert_eq!(size(&undo)[1], one_row, "one row, as Adjust's: {undo}");
    let tray = ui.rect("tray");
    assert!(tray[2] - tray[0] < 190.0, "{tray:?}");
    let x = |i: usize| undo["rect"][i].as_f64().unwrap_or_default();
    assert!(
        x(0) >= tray[0] && x(2) <= tray[2],
        "inside the tray {tray:?}: {undo}"
    );
}

/// History's Adjust runs `ui.adjust` `{ open = true }` (decision 0060 as amended for H2,
/// section 12): it opens the adjust panel again for the newest step a menu ran; once a stroke
/// is the newest it opens nothing, and the status line says there is nothing to adjust.
#[test]
fn adjust_opens_the_panel_for_the_newest_menu_run_or_says_why_not() {
    let mut ui = Ui::new();
    ui.ok(
        "primitive.sphere",
        json!({ "name": "ball", "subdivisions": 3 }),
    );
    // The part keeps this cell size, so the menu's run, which gives no resolution, is quick.
    ui.ok("mesh.voxel_remesh", json!({ "resolution": 24 }));
    crate::header::open(&mut ui, "Mesh");
    ui.press("menu > Voxel Remesh");
    let panel = |ui: &mut Ui| crate::header::panel(ui, "adjust");
    assert!(panel(&mut ui).is_some(), "the menu's run opens it");
    ui.press("adjust > Close");
    let adjust = button(&mut ui, "Adjust");
    let runs = json!({ "command": "ui.adjust", "args": { "open": true } });
    assert_eq!(adjust["command"], runs);
    ui.press("tray > Adjust");
    let shown = panel(&mut ui).expect("Adjust opens it again");
    assert_eq!(shown["title"], "Voxel remesh");
    let stroke = json!({ "brush": "Draw", "size": "20%", "samples": [{ "position": [0, 0, 1] }] });
    ui.ok("brush.stroke", stroke);
    ui.settle();
    assert!(panel(&mut ui).is_none(), "the stroke is the newest step");
    ui.press("tray > Adjust");
    assert!(panel(&mut ui).is_none(), "nothing to open");
    let status = ui.labels("status");
    let said = status.iter().any(|l| l == "Problem: Nothing to adjust");
    assert!(said, "{status:?}");
}
