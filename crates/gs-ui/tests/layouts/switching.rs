//! Arranging the window for the other hand is a command and changes only how the window looks
//! (the trays swap and the axis figure moves with the pen side): the scene, the brush, the
//! history and the keymap stay exactly as they were. A layout name
//! that does not exist is refused with the ones that do.

use gs_protocol::ErrorCode;
use serde_json::{Map, Value, json};

use crate::rig::Ui;

/// Every state value but the layout's.
fn sculpting(ui: &mut Ui) -> Map<String, Value> {
    let mut state = ui.state().as_object().cloned().unwrap_or_default();
    state.retain(|name, _| !name.starts_with("layout"));
    state
}

/// The left edge of the panel `id`, in points.
fn left_edge(ui: &mut Ui, id: &str) -> f64 {
    let d = ui.describe();
    let p = (d["panels"].as_array().into_iter().flatten())
        .find(|p| p["id"] == id)
        .cloned();
    p.map_or(0.0, |p| p["rect"][0].as_f64().unwrap_or_default())
}

#[test]
fn the_left_hand_mirrors_the_window_and_keeps_the_scene_the_brush_and_the_keymap() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 12 }));
    let stroke = json!({ "brush": "Draw", "size": "20%", "samples": [{ "position": [0, 0, 1] }] });
    ui.ok("brush.stroke", stroke);
    let settings = json!({ "brush": "Clay Build", "size_px": 40, "strength": 0.4, "falloff": 30 });
    ui.ok("brush.set", settings);
    ui.ok("brush.set", json!({ "flip_mirror": "x" }));
    let before = (ui.session.hash(), sculpting(&mut ui), ui.session.history());
    let keymap = ui.host.lock().shell.keymap.clone();
    assert_eq!(ui.layouts(), ["workbench"], "one interface");
    assert!(
        left_edge(&mut ui, "brush") < left_edge(&mut ui, "tray"),
        "right hand: the brush tray on the left"
    );
    // The axis figure sits in the view's top corner on the pen side.
    let beside = |ui: &mut Ui| {
        let (view, figure) = (ui.rect("viewport"), ui.rect("figure"));
        let gap = |a: f64, b: f64| (a - b).abs() < 30.0;
        (gap(figure[2], view[2]), gap(figure[0], view[0]))
    };
    assert_eq!(beside(&mut ui), (true, false), "right hand: top right");
    let left = ui.ok("ui.layout", json!({ "name": "workbench", "hand": "left" }));
    assert_eq!(left["hand"], json!("left"));
    assert!(
        left_edge(&mut ui, "brush") > left_edge(&mut ui, "tray"),
        "left hand: swapped"
    );
    assert_eq!(beside(&mut ui), (false, true), "left hand: top left");
    let after = (ui.session.hash(), sculpting(&mut ui), ui.session.history());
    assert_eq!(after, before, "scene, brush and history unchanged");
    assert_eq!(ui.host.lock().shell.keymap, keymap, "the keymap too");
    let saved = ui.host.lock().shell.layout.layout.clone();
    assert_eq!(
        (saved.current.as_str(), saved.hand),
        ("workbench", gs_engine::Hand::Left),
        "the state the layout file saves"
    );
    let (code, message) = ui.err("ui.layout", json!({ "name": "cockpit" }));
    assert_eq!(code, ErrorCode::NotFound);
    assert!(message.contains("there are workbench"), "{message}");
    ui.ok("history.undo", json!({}));
    assert_ne!(ui.session.hash(), before.0, "undo still undoes the stroke");
}
