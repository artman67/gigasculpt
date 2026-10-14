//! The Gizmo's deformers in Brush tray > Brush (decision 0041, amended 2026-10-12, step `DF7b`):
//! each pick opens its deformer on the part at no change (a curve along the part's longest
//! side), a settings row changes it with one
//! `update` and no undo step and the pen's lift lets go of it, Accept keeps it as one step named
//! after it, Cancel puts every bit back and Reset puts the settings back with it still open.
//! Enter and Escape do as Accept and Cancel unless something else takes the key, the quick menu
//! has the same three, picking another tool accepts, Ctrl+S during a Deform pull saves once the
//! pen lifts, and the status line says autosave waits for the open deformer.

use gs_platform::{Input, Modifiers};
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, folder, held, key};

/// A sphere of radius 0.7 with the Gizmo tool on, so the Brush tray shows the Gizmo's set.
fn gizmo(ui: &mut Ui) {
    ui.ok("primitive.sphere", json!({ "radius": 0.7 }));
    ui.ok("ui.gizmo", json!({ "tool": true }));
    ui.settle();
}

/// Sends `inputs` through the studio and draws one frame: everything the window asked for and
/// still waiting to run with the next [`Ui::settle`], earlier frames' steps included.
fn asked(ui: &mut Ui, inputs: &[Input]) -> Vec<Step> {
    let mut studio = ui.host.lock();
    studio.input(inputs, Modifiers::default());
    studio.frame().unwrap_or_else(|e| panic!("{e}"));
    let steps = studio.shell.take_commands();
    for step in &steps {
        studio.shell.queue(step.clone());
    }
    steps
}

/// Types `text` into the Brush tray's slider `name`, whose field a tap on its value opens, and
/// Enter, then runs what that asked for. Returns the `gizmo.deformer` phases asked for on the
/// frames of the typing and of Enter's press and release.
fn type_value(ui: &mut Ui, name: &str, text: &str) -> Vec<String> {
    let r = ui.rect(&format!("brush > {name}"));
    ui.tap_at(r[2] - 4.0, middle(r)[1]);
    let none = Modifiers::default();
    let inputs = [
        Input::Text(text.into()),
        key("Return", true, none),
        key("Return", false, none),
    ];
    let mut steps = Vec::new();
    for input in inputs {
        steps = asked(ui, &[input]);
    }
    ui.settle();
    (steps.iter())
        .filter(|s| s.command == "gizmo.deformer")
        .map(|s| s.args["phase"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The open deformer's first call's operation, as `session.state` reports it.
fn op(ui: &mut Ui) -> Value {
    ui.state()["deformer.transforms"][0]["op"].clone()
}

fn undos(ui: &mut Ui) -> u64 {
    ui.state()["history.undos"].as_u64().unwrap_or_default()
}

/// Smooth bend picked and bent 90 degrees from its Angle row.
fn bent(ui: &mut Ui) {
    ui.press("brush > Smooth bend");
    type_value(ui, "Angle", "90");
    assert_eq!(op(ui)["bend"]["degrees"], json!(90.0), "{}", ui.state());
}

const THREE: [&str; 3] = ["Accept", "Cancel", "Reset"];

#[test]
fn smooth_bend_opens_one_bend_at_no_change_on_the_gizmo_part_and_lights() {
    let mut ui = Ui::new();
    gizmo(&mut ui);
    let (hash, before) = (ui.session.hash(), undos(&mut ui));
    for label in THREE {
        assert!(!ui.lists("brush", label), "no {label} before a pick");
    }
    ui.press("brush > Smooth bend");

    let state = ui.state();
    assert_eq!(state["deformer"], "bend", "{state}");
    assert_eq!(state["deformer.part"], "sphere");
    let transforms = state["deformer.transforms"].as_array().cloned();
    let transforms = transforms.unwrap_or_default();
    assert_eq!(transforms.len(), 1, "{transforms:?}");
    let bend = &transforms[0]["op"]["bend"];
    assert_eq!(
        (&bend["axis"], &bend["toward"], &bend["degrees"]),
        (&json!("x"), &json!("y"), &json!(0.0))
    );
    assert!(transforms[0]["box"].is_object(), "{:?}", transforms[0]);
    assert_eq!(undos(&mut ui), before, "a pick is no step");
    assert_eq!(ui.session.hash(), hash, "nothing moves at no change");
    let widgets = ui.widgets("brush");
    let lit = |label: &str| {
        widgets
            .iter()
            .any(|w| w["label"] == label && w["selected"] == true)
    };
    assert!(lit("Smooth bend") && !lit("Twist"), "{widgets:?}");
    for label in THREE.into_iter().chain(["Angle", "Axis X", "Toward Y"]) {
        assert!(ui.lists("brush", label), "{label} while open");
    }
}

/// Each of the picker's buttons and the deformer it opens.
const PICKS: [(&str, &str); 15] = [
    ("Offset", "move"),
    ("Rotate", "rotate"),
    ("Stretch", "scale"),
    ("Fold bend", "fold"),
    ("Smooth bend", "bend"),
    ("Twist", "twist"),
    ("Taper", "taper"),
    ("Flatten", "clip"),
    ("Spherize", "spherize"),
    ("Inflate", "inflate"),
    ("Noise", "noise"),
    ("Skew", "shear"),
    ("Soft cage", "lattice"),
    ("Hard cage", "lattice"),
    ("Curve", "curve"),
];

#[test]
fn every_pick_opens_its_deformer_at_no_change_and_a_curve_runs_along_the_longest_side() {
    let mut ui = Ui::new();
    let tall = json!({ "radius": 0.3, "height": 2, "axis": "z" });
    ui.ok("primitive.cylinder", tall);
    ui.ok("ui.gizmo", json!({ "tool": true }));
    ui.settle();
    let (hash, before) = (ui.session.hash(), undos(&mut ui));
    for (button, kind) in PICKS {
        ui.press(&format!("brush > {button}"));
        let state = ui.state();
        assert_eq!(state["deformer"], kind, "{button}: {state}");
        if button == "Hard cage" {
            assert_eq!(op(&mut ui)["lattice"]["kind"], "hard");
        }
    }
    assert_eq!(
        op(&mut ui)["curve"]["axis"],
        "z",
        "along the cylinder's height"
    );
    assert_eq!(ui.session.hash(), hash, "no pick moves a point");
    assert_eq!(undos(&mut ui), before, "nor makes a step");
}

#[test]
fn a_settings_change_is_one_update_and_no_step_and_the_lift_lets_go() {
    let mut ui = Ui::new();
    gizmo(&mut ui);
    ui.press("brush > Smooth bend");
    let (hash, before) = (ui.session.hash(), undos(&mut ui));

    let phases = type_value(&mut ui, "Angle", "90");
    let updates = phases.iter().filter(|p| *p == "update").count();
    assert_eq!(updates, 1, "{phases:?}");
    let state = ui.state();
    assert_eq!(
        state["deformer.transforms"][0]["op"]["bend"]["degrees"],
        json!(90.0)
    );
    assert_eq!(undos(&mut ui), before, "no undo step until Accept");
    assert_ne!(ui.session.hash(), hash, "the part bends live");
    assert_eq!(state["deformer.held"], json!(false), "the lift: {state}");
    assert_eq!(state["session.gesture"], json!(false), "{state}");
    let after = asked(&mut ui, &[]);
    let deformer = after.iter().any(|s| s.command == "gizmo.deformer");
    assert!(!deformer, "no other lift or update: {after:?}");
}

#[test]
fn accept_keeps_one_step_named_bend_and_the_rows_go() {
    let mut ui = Ui::new();
    gizmo(&mut ui);
    let before = undos(&mut ui);
    bent(&mut ui);
    ui.press("brush > Accept");

    let state = ui.state();
    assert_eq!(state["deformer"], Value::Null, "{state}");
    assert_eq!(state["history.undo"], "Bend");
    assert_eq!(undos(&mut ui), before + 1);
    for label in THREE.into_iter().chain(["Angle"]) {
        assert!(!ui.lists("brush", label), "{label} goes");
    }
}

#[test]
fn cancel_puts_every_bit_back_and_reset_keeps_it_open_at_no_change() {
    let mut ui = Ui::new();
    gizmo(&mut ui);
    let (hash, before) = (ui.session.hash(), undos(&mut ui));
    bent(&mut ui);
    ui.press("brush > Reset");
    assert_eq!(ui.state()["deformer"], "bend", "Reset keeps it open");
    assert_eq!(op(&mut ui)["bend"]["degrees"], json!(0.0));
    assert_eq!(ui.session.hash(), hash, "at no change");

    type_value(&mut ui, "Angle", "45");
    ui.press("brush > Cancel");
    assert_eq!(ui.state()["deformer"], Value::Null);
    assert_eq!(ui.session.hash(), hash, "bit for bit");
    assert_eq!(undos(&mut ui), before, "no step");
}

#[test]
fn enter_accepts_and_escape_cancels_but_not_during_a_held_slider_key() {
    let mut ui = Ui::new();
    gizmo(&mut ui);
    let none = Modifiers::default();
    bent(&mut ui);
    ui.tap("Return", none);
    assert_eq!(ui.state()["deformer"], Value::Null, "Enter accepts");
    assert_eq!(ui.state()["history.undo"], "Bend");
    let accepted = ui.session.hash();

    bent(&mut ui);
    ui.tap("Escape", none);
    assert_eq!(ui.state()["deformer"], Value::Null, "Escape cancels");
    assert_eq!(ui.session.hash(), accepted, "bit for bit");

    bent(&mut ui);
    let [x, y] = ui.middle();
    ui.pen(x, y);
    ui.input(&[key("S", true, none)], none);
    ui.tap("Escape", none);
    assert_eq!(ui.state()["deformer"], "bend", "Escape lets go of S only");
    ui.input(&[key("S", false, none)], none);
    assert_eq!(ui.state()["deformer"], "bend");
}

#[test]
fn the_quick_menus_accept_cancel_and_reset_do_as_the_trays() {
    let mut ui = Ui::new();
    gizmo(&mut ui);
    // Open, or shut so a press in the tray is not the press outside that shuts it.
    let quick = |ui: &mut Ui, open: bool| {
        ui.ok("ui.quick", json!({ "open": open }));
        ui.settle();
    };
    quick(&mut ui, true);
    for label in THREE {
        assert!(!ui.lists("quick", label), "no {label} with none open");
    }
    quick(&mut ui, false);
    let hash = ui.session.hash();
    bent(&mut ui);
    quick(&mut ui, true);
    let labels = ui.labels("quick");
    for label in THREE {
        let shown = labels.iter().filter(|l| *l == label).count();
        assert_eq!(
            shown, 1,
            "{label} once, though the menu holds the Gizmo's set: {labels:?}"
        );
    }
    ui.press("quick > Reset");
    assert_eq!(op(&mut ui)["bend"]["degrees"], json!(0.0), "Reset");
    assert_eq!(ui.session.hash(), hash);

    quick(&mut ui, false);
    type_value(&mut ui, "Angle", "90");
    quick(&mut ui, true);
    ui.press("quick > Cancel");
    assert_eq!(ui.state()["deformer"], Value::Null, "Cancel");
    assert_eq!(ui.session.hash(), hash, "bit for bit");

    quick(&mut ui, false);
    bent(&mut ui);
    quick(&mut ui, true);
    ui.press("quick > Accept");
    assert_eq!(ui.state()["deformer"], Value::Null, "Accept");
    assert_eq!(ui.state()["history.undo"], "Bend");
}

#[test]
fn picking_another_tool_accepts_the_open_deformer() {
    let mut ui = Ui::new();
    gizmo(&mut ui);
    let before = undos(&mut ui);
    bent(&mut ui);
    ui.press("shelf > Gizmo");
    let state = ui.state();
    assert_eq!(state["gizmo.tool"], json!(false), "{state}");
    assert_eq!(state["deformer"], Value::Null);
    assert_eq!(state["history.undo"], "Bend");
    assert_eq!(undos(&mut ui), before + 1);
}

#[test]
fn ctrl_s_during_a_deform_pull_saves_once_the_pen_lifts() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "radius": 0.7 }));
    crate::header::open(&mut ui, "Deform");
    let path = folder("gizmo-deformer-save").join("scene.gss");
    ui.ok("file.save", json!({ "path": path }));
    let saved = std::fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
    let r = ui.rect("menu > Smooth bend");
    let [x, y] = [r[0] + 20.0, middle(r)[1]];
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x + 40.0, y, true);
    assert_eq!(
        ui.state()["deformer.held"],
        json!(true),
        "the pull holds it"
    );

    ui.tap("S", held("Ctrl"));
    ui.input(&[], Modifiers::default());
    let words = "Saves when the pen lifts";
    assert!(ui.lists("status", words), "{:?}", ui.labels("status"));
    assert_eq!(std::fs::read(&path).ok(), Some(saved.clone()), "not yet");
    ui.touch(x + 40.0, y, false);
    assert_eq!(ui.state()["history.undo"], "Bend", "the pull landed");
    assert_ne!(
        std::fs::read(&path).ok(),
        Some(saved),
        "saved after the lift"
    );
    assert!(!ui.lists("status", words), "{:?}", ui.labels("status"));
}

#[test]
fn the_status_line_says_autosave_waits_while_a_deformer_is_open() {
    let dir = folder("gizmo-deformer-autosave");
    let mut ui = Ui::launch(&dir, &dir.join("state"));
    ui.settle();
    gizmo(&mut ui);
    let words = "Autosave waits for the open deformer";
    assert!(!ui.lists("status", words), "none open");
    ui.press("brush > Smooth bend");
    assert!(ui.lists("status", words), "{:?}", ui.labels("status"));
    ui.press("brush > Accept");
    assert!(!ui.lists("status", words), "{:?}", ui.labels("status"));
}
