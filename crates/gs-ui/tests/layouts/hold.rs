//! A header view button with options (decision 0025, rule 8.5; decision 0060 as amended for H5,
//! the strip's buttons in the header's pen end): a tap on Frame frames every
//! part; a press held past a click, or a barrel click, opens its options (what else it frames)
//! instead, and letting go of the held press runs nothing. Isolate likewise: a tap shows the
//! active part alone, or ends Isolate while it is, and a held press opens a button per part.

use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::Ui;

/// The targets of the open Frame options' buttons; empty while they are shut.
fn targets(ui: &mut Ui) -> Vec<Value> {
    let widgets = ui.widgets("frame");
    let runs = widgets
        .iter()
        .filter(|w| w["command"]["command"] == "view.frame");
    runs.map(|w| w["command"]["args"]["target"].clone())
        .collect()
}

/// A click of the pen's barrel button at (`x`, `y`), hovering.
fn barrel(ui: &mut Ui, x: f64, y: f64) {
    for buttons in [1, 0] {
        let sample = json!({ "samples": [{ "x": x, "y": y, "buttons": buttons }] });
        ui.ok("pen.input", sample);
        ui.settle();
    }
}

#[test]
fn a_tap_on_frame_frames_and_a_held_press_or_a_barrel_click_opens_its_options() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    let [x, y] = middle(ui.rect("header > Frame"));
    let before = ui.logged("view.frame").len();
    ui.tap_at(x, y);
    assert_eq!(ui.logged("view.frame").len(), before + 1, "a tap frames");
    assert!(targets(&mut ui).is_empty(), "a tap leaves its options shut");

    // Held a second, still: the options open while the pen still presses, with no input after
    // the press, and letting go frames nothing.
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.wait(1.0);
    let all = ["mask", "unmasked", "last-edit"].map(Value::from);
    assert_eq!(targets(&mut ui), all, "held, its options open");
    ui.touch(x, y, false);
    assert_eq!(targets(&mut ui), all, "they stay open when the pen lets go");
    assert_eq!(
        ui.logged("view.frame").len(),
        before + 1,
        "a held press frames nothing"
    );

    barrel(&mut ui, x, y);
    assert!(targets(&mut ui).is_empty(), "a barrel click shuts them");
    barrel(&mut ui, x, y);
    assert_eq!(targets(&mut ui), all, "and opens them");

    let info = ui.ok("ui.palette", json!({ "query": "Frame mask" }));
    let rows = info["rows"].as_array().cloned().unwrap_or_default();
    let row = rows.iter().find(|r| r["label"] == "Frame mask");
    let places = row.map(|r| r["places"].clone());
    let under = json!(["Header > Frame"]);
    assert_eq!(places, Some(under), "the palette says where it lives");
}

/// The parts `part.list` says show, in list order, and the active one.
fn showing(ui: &mut Ui) -> (Vec<String>, String) {
    let list = ui.ok("part.list", json!({}))["parts"].clone();
    let list = list.as_array().cloned().unwrap_or_default();
    let name = |p: &Value| p["name"].as_str().unwrap_or_default().to_string();
    let shown = list.iter().filter(|p| p["shown"] == json!(true));
    let active = list.iter().find(|p| p["active"] == json!(true));
    (
        shown.map(name).collect(),
        active.map(name).unwrap_or_default(),
    )
}

#[test]
fn a_tap_on_isolate_shows_the_active_part_alone_and_a_held_press_opens_a_button_per_part() {
    let mut ui = Ui::new();
    showing(&mut ui);
    ui.settle();
    let header = ui.widgets("header");
    let isolate = header.iter().find(|w| w["label"] == "Isolate");
    let enabled = isolate.map(|w| w["enabled"].clone());
    assert_eq!(
        enabled,
        Some(json!(false)),
        "no parts, no active part to isolate"
    );
    ui.ok("primitive.sphere", json!({}));
    ui.ok("primitive.cube", json!({}));
    ui.ok("part.select", json!({ "part": "sphere" }));
    let (all, active) = showing(&mut ui);
    assert_eq!(all, ["sphere", "cube"]);
    assert_eq!(active, "sphere");
    let before = ui.logged("part.isolate").len();

    ui.press("header > Isolate");
    assert_eq!(
        showing(&mut ui).0,
        ["sphere"],
        "a tap shows the active part alone"
    );
    assert!(
        ui.widgets("isolate").is_empty(),
        "a tap leaves its options shut"
    );
    ui.press("header > Isolate");
    assert_eq!(showing(&mut ui).0, all, "a second tap ends Isolate");

    ui.hold("header > Isolate");
    let labels = ui.labels("isolate");
    for part in ["sphere", "cube", "Dynamic"] {
        assert!(
            labels.iter().any(|l| l == part),
            "held, it shows {part}: {labels:?}"
        );
    }
    assert_eq!(showing(&mut ui).0, all, "a held press isolates nothing");
    assert_eq!(
        ui.logged("part.isolate").len(),
        before + 2,
        "only the two taps ran"
    );
}
