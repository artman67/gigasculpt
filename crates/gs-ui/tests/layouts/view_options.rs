//! The header's view buttons' options in compact rows (decision 0060, section 10, as amended
//! for R1): Frame's, Wire's and Floor's choices share a row by their short labels, a choice of
//! a few values lit on the one in use; a row shows only while it applies; and each button runs
//! what it ran under its long label, with the same key.

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::gizmo_deformer::shows as has;
use crate::rig::Ui;

/// The labels on the row of panel `id`'s widget labelled `label`, left to right.
pub fn row(ui: &mut Ui, id: &str, label: &str) -> Vec<String> {
    let widgets = ui.widgets(id);
    let edge = |w: &Value, i: usize| w["rect"][i].as_f64().unwrap_or_default();
    let middle = |w: &Value| f64::midpoint(edge(w, 1), edge(w, 3));
    let at = widgets.iter().find(|w| w["label"] == label);
    let at = middle(at.unwrap_or_else(|| panic!("{id} > {label} shows: {widgets:?}")));
    let mut on: Vec<&Value> = (widgets.iter())
        .filter(|w| w["role"] != "scroll bar" && (middle(w) - at).abs() < 3.0)
        .collect();
    on.sort_by(|a, b| edge(a, 0).total_cmp(&edge(b, 0)));
    (on.iter())
        .filter_map(|w| Some(w["label"].as_str()?.to_string()))
        .collect()
}

/// Which of `labels` are lit in panel `id`.
pub fn lit(ui: &mut Ui, id: &str, labels: &[&str]) -> Vec<String> {
    let widgets = ui.widgets(id);
    let lit = widgets.iter().filter(|w| w["selected"] == true);
    (lit.filter_map(|w| w["label"].as_str()))
        .filter(|l| labels.contains(l))
        .map(str::to_string)
        .collect()
}

/// Holds the header's view button `button` until its options open, which `ui.describe` lists
/// as panel `id`, titled with the button's label (decision 0060 as amended for H5).
fn open(ui: &mut Ui, id: &str, button: &str) {
    ui.hold(&format!("header > {button}"));
    let d = ui.describe();
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let title = (panels.iter().find(|p| p["id"] == id)).map(|p| p["title"].clone());
    assert_eq!(title, Some(json!(button)), "the {id} options open");
}

#[test]
fn the_floor_planes_and_fill_are_a_row_each_lit_on_what_is_in_use() {
    const FILL: [&str; 4] = ["Opaque", "Light", "Strong", "Facing"];
    let mut ui = Ui::new();
    ui.ok("view.display", json!({ "floor": true }));
    open(&mut ui, "floor", "Floor");
    assert_eq!(row(&mut ui, "floor", "Planes"), ["Planes", "X", "Y", "Z"]);
    assert_eq!(
        lit(&mut ui, "floor", &["X", "Y", "Z"]),
        ["Y"],
        "the floor's own"
    );
    let fill = row(&mut ui, "floor", "Fill");
    assert_eq!(fill, ["Fill", "Opaque", "Light", "Strong", "Facing"]);
    assert_eq!(lit(&mut ui, "floor", &FILL), ["Opaque"]);
    ui.press("floor > Strong");
    assert_eq!(ui.state()["view.floor.fill"], json!(2));
    assert_eq!(
        lit(&mut ui, "floor", &FILL),
        ["Strong"],
        "lit on the one in use"
    );
}

#[test]
fn the_floor_rows_show_only_while_they_apply() {
    let mut ui = Ui::new();
    open(&mut ui, "floor", "Floor");
    assert!(
        has(&mut ui, "floor", "Show floor"),
        "hidden, it can be shown"
    );
    assert!(!has(&mut ui, "floor", "Planes"), "hidden, its rows wait");
    ui.press("floor > Show floor");
    assert_eq!(ui.state()["view.floor"], json!(true));
    // Sliders are listed by their long labels: Elevation reads Height, Grid size Size.
    for shows in [
        "Planes",
        "Snap",
        "Elevation",
        "Grid size",
        "Fill",
        "Axis colors",
    ] {
        assert!(has(&mut ui, "floor", shows), "{shows} with the floor");
    }
    assert!(
        !has(&mut ui, "floor", "Show floor"),
        "shown, its switch is the header's"
    );
    assert!(
        !has(&mut ui, "floor", "Tiles"),
        "an endless grid has no tiles"
    );
    ui.press("floor > Snap");
    assert!(
        !has(&mut ui, "floor", "Elevation"),
        "snapped, its height is unused"
    );
    ui.press("floor > Snap");
    assert!(has(&mut ui, "floor", "Elevation"), "back with Snap off");
    ui.ok("view.floor", json!({ "size": 2.0 }));
    ui.settle();
    assert!(has(&mut ui, "floor", "Tiles"), "a sized grid's tiles");
}

#[test]
fn the_wire_switches_share_a_row_and_opacity_shows_while_edges_are_on() {
    let mut ui = Ui::new();
    open(&mut ui, "wire", "Wire");
    let switches = row(&mut ui, "wire", "Edges");
    assert_eq!(switches, ["Edges", "Fill", "Density", "Dots"]);
    assert!(has(&mut ui, "wire", "Edge opacity"));
    ui.press("wire > Edges");
    assert_eq!(ui.state()["view.wire.line"], json!(false));
    assert!(
        !has(&mut ui, "wire", "Edge opacity"),
        "no edges, no opacity"
    );
    ui.press("wire > Edges");
    assert!(has(&mut ui, "wire", "Edge opacity"), "back with the edges");
}

/// Whether widget `w` runs `command` with `args`, a `"flip"` among them as a true or false.
fn runs(w: &Value, command: &str, args: &Value) -> bool {
    let (Some(got), Some(want)) = (w["command"]["args"].as_object(), args.as_object()) else {
        return false;
    };
    let arg = |(k, v): (&String, &Value)| match v.as_str() {
        Some("flip") => got.get(k).is_some_and(Value::is_boolean),
        _ => got.get(k) == Some(v),
    };
    w["command"]["command"] == command && got.len() == want.len() && want.iter().all(arg)
}

/// Each compact button of panel `id`: its label, what its long button ran, and its key.
fn check(ui: &mut Ui, id: &str, buttons: &[(&str, &str, Value, Option<&str>)]) {
    let widgets = ui.widgets(id);
    for (label, command, args, key) in buttons {
        let w = widgets.iter().find(|w| w["label"] == *label);
        let w = w.unwrap_or_else(|| panic!("{id} > {label} shows: {widgets:?}"));
        assert!(
            runs(w, command, args),
            "{id} > {label} runs {command} {args}: {w}"
        );
        assert_eq!(w["key"], json!(key), "{id} > {label}'s key");
    }
}

#[test]
fn each_compact_button_runs_what_its_long_button_ran_with_its_key() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    open(&mut ui, "frame", "Frame");
    let frame = row(&mut ui, "frame", "Mask");
    assert_eq!(frame, ["Mask", "Unmasked", "Last edit"], "Frame's row");
    let target = |t: &str| json!({ "target": t });
    let frames = [
        ("Mask", "view.frame", target("mask"), Some("Alt+F")),
        ("Unmasked", "view.frame", target("unmasked"), None),
        (
            "Last edit",
            "view.frame",
            target("last-edit"),
            Some("Period"),
        ),
    ];
    check(&mut ui, "frame", &frames);
    ui.tap("Escape", Modifiers::default());

    open(&mut ui, "wire", "Wire");
    let flip = |arg: &str| json!({ arg: "flip" });
    let wires = [
        ("Edges", "view.wire", flip("line"), None),
        ("Fill", "view.wire", flip("fill"), None),
        ("Density", "view.wire", flip("density"), None),
        ("Dots", "view.wire", flip("dots"), None),
    ];
    check(&mut ui, "wire", &wires);
    ui.tap("Escape", Modifiers::default());

    open(&mut ui, "floor", "Floor");
    // Show floor is new: it sets the floor on, so it is not a second home of the Floor
    // button's flip (decision 0025 rule 8.1); while the floor is hidden, Shift+P's flip does
    // the same, so it shows that key.
    let on = json!({ "floor": true });
    let shows = [("Show floor", "view.display", on, Some("Shift+P"))];
    check(&mut ui, "floor", &shows);
    ui.press("floor > Show floor");
    let fill = |n: u8| json!({ "fill": n });
    let floors = [
        ("X", "view.floor", flip("x"), None),
        ("Y", "view.floor", flip("y"), None),
        ("Z", "view.floor", flip("z"), None),
        ("Snap", "view.floor", flip("snap"), None),
        ("Opaque", "view.floor", fill(0), None),
        ("Light", "view.floor", fill(1), None),
        ("Strong", "view.floor", fill(2), None),
        ("Facing", "view.floor", fill(3), None),
    ];
    check(&mut ui, "floor", &floors);
}
