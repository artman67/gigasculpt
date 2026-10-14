//! The stroke sliders in Brush tray > Stroke and the brush's ⋯ in the tray's header (decision
//! 0036 section 4, decision 0018 "Saving a brush", decision 0060 section 6): Steady and Spacing
//! each set their stroke setting once, when the pen lifts; Save brush
//! asks for a file and writes the brush there, asking before it replaces one, and Load brush,
//! asking from that folder, brings the brush's settings back; Reset to defaults puts the brush
//! back as it ships, and Copy settings to… gives another brush its changes.

use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, folder};
use crate::scene::answer;

/// Drags the brush tray's slider `label` from its left end to its right with the pen. Returns
/// the state value `shows` before the touch, while the pen is still down, and after the lift.
fn drag(ui: &mut Ui, label: &str, shows: &str) -> [Value; 3] {
    let start = ui.state()[shows].clone();
    let r = ui.rect(&format!("brush > {label}"));
    let y = middle(r)[1];
    ui.touch(r[0] + 2.0, y, false);
    ui.touch(r[0] + 2.0, y, true);
    ui.touch(r[2] - 2.0, y, true);
    let down = ui.state()[shows].clone();
    ui.touch(r[2] - 2.0, y, false);
    [start, down, ui.state()[shows].clone()]
}

#[test]
fn smooth_and_step_set_the_strings_lag_and_the_dab_spacing_once_when_the_pen_lifts() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    // Steady, which shows while Stabilize is on, spans 0 to 0.99, Spacing 0.01 to 4 brush
    // diameters: dragged to the right end.
    let on = json!({ "stroke": { "smoothing": { "radius": 1.0 } } });
    ui.ok("brush.set", json!({ "settings": on }));
    for (label, shows, most) in [
        ("Steady", "brush.smoothing_factor", 0.9),
        ("Spacing", "brush.spacing", 3.5),
    ] {
        let [start, down, set] = drag(&mut ui, label, shows);
        assert_eq!(down, start, "{label}: nothing set while the pen is down");
        assert!(set.as_f64().is_some_and(|v| v > most), "{label}: {set}");
    }
}

/// The palette's form, if one is open: its command and each field with the text it starts with.
fn form(ui: &Ui) -> Option<(String, Vec<(String, String)>)> {
    let studio = ui.host.lock();
    let form = studio.shell.palette.form.as_ref()?;
    Some((form.step.command.clone(), form.fields.clone()))
}

/// Picks `pick` from the brush's ⋯, in the tray's header.
fn menu(ui: &mut Ui, pick: &str) {
    ui.press("brush > ···");
    ui.press(&format!("brush > {pick}"));
}

fn spaced(ui: &mut Ui, spacing: f64) {
    ui.ok(
        "brush.set",
        json!({ "settings": { "stroke": { "spacing": spacing } } }),
    );
}

#[test]
fn save_brush_asks_for_a_file_then_before_replacing_it_and_load_brush_brings_it_back() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    assert!(
        !ui.labels("shelf").iter().any(|l| l == "Save"),
        "off the shelf"
    );
    let dir = folder("brush-file");
    spaced(&mut ui, 0.5);
    menu(&mut ui, "Save brush…");
    let empty = ("path".to_string(), String::new());
    assert_eq!(form(&ui), Some(("brush.save".into(), vec![empty])));
    let path = dir.join("wide-draw.toml");
    answer(&mut ui, &path.display().to_string());
    assert!(path.is_file(), "Save brush wrote {}", path.display());
    // Saved again to the same file, the form starting in its folder: a plain question, and its
    // yes replaces the file.
    spaced(&mut ui, 0.75);
    menu(&mut ui, "Save brush…");
    answer(&mut ui, "wide-draw.toml");
    ui.press("palette > Overwrite");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(text.contains("0.75"), "replaced: {text}");
    spaced(&mut ui, 0.1);
    menu(&mut ui, "Load brush…");
    let start = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
    let asked = ("path".to_string(), start);
    assert_eq!(
        form(&ui),
        Some(("brush.load".into(), vec![asked])),
        "in that folder"
    );
    answer(&mut ui, "wide-draw.toml");
    assert_eq!(
        ui.state()["brush.spacing"],
        json!(0.75),
        "the Step comes back"
    );
}

#[test]
fn reset_to_defaults_and_copy_settings_to_work_on_the_brushs_changes() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let shipped = ui.state()["brush.spacing"].clone();
    spaced(&mut ui, 0.5);
    menu(&mut ui, "Copy settings to…");
    assert_eq!(form(&ui).map(|f| f.0), Some("brush.set".into()));
    answer(&mut ui, "Clay");
    assert_eq!(ui.state()["brush"], "Draw", "the pen keeps its brush");
    menu(&mut ui, "Reset to defaults");
    assert_eq!(ui.state()["brush.spacing"], shipped, "Draw as it ships");
    ui.ok("brush.set", json!({ "brush": "Clay" }));
    assert_eq!(ui.state()["brush.spacing"], json!(0.5), "Clay took it");
}
