//! The default keys against the catalog's tables (`interface.default-bindings` and its -view,
//! -sculpt, -mask and -geometry tables): each bound row's key asks for its command in its
//! context, the keys sheet lists every key once under its context, Tab hides and shows the
//! panels without taking the keyboard from the keymap, and each table's Check runs through the
//! keys. Rows whose command does not exist yet are reserved, not bound, and are not here.

use gs_platform::Modifiers;
use gs_protocol::{AttrValues, PlainMesh, Step};
use gs_ui::Keymap;
use serde_json::{Value, json};

use crate::rig::{Ui, key};

/// Every bound row: its key as the keymap writes it, its context (the catalog table, and the
/// heading the keys sheet lists it under), its command, and the arguments it asks for in a new
/// window (a `"flip"` filled in as that window shows: panels showing, the Library shut, the
/// mask shown and the floor hidden, no smoothing). `"drag"` stands for the number a held key
/// sets.
const ROWS: &[(&str, &str, &str, &str)] = &[
    // interface.default-bindings: the general rows, and ours.
    ("Space", "General", "ui.palette", "{}"),
    ("Shift+Space", "General", "ui.quick", r#"{"open": true}"#),
    ("Ctrl+Z", "General", "history.undo", "{}"),
    ("Ctrl+Shift+Z", "General", "history.redo", "{}"),
    ("Ctrl+Y", "General", "history.redo", "{}"),
    ("Ctrl+S", "General", "file.save", "{}"),
    (
        "Ctrl+O",
        "General",
        "ui.palette",
        r#"{"form": "file.open", "ask": ["path"]}"#,
    ),
    (
        "Ctrl+Shift+T",
        "General",
        "ui.palette",
        r#"{"form": "file.save", "args": {"parts": ["@active"]}, "ask": ["path"]}"#,
    ),
    (
        "Ctrl+N",
        "General",
        "ui.palette",
        r#"{"form": "file.new", "ask": ["discard_changes"]}"#,
    ),
    (
        "Ctrl+Alt+R",
        "General",
        "ui.palette",
        r#"{"form": "file.revert"}"#,
    ),
    ("Ctrl+Alt+S", "General", "file.save_next", "{}"),
    ("9", "General", "file.quick_save", "{}"),
    ("Tab", "General", "ui.layout", r#"{"chrome": false}"#),
    ("Comma", "General", "ui.library", r#"{"open": true}"#),
    ("F2", "General", "ui.wheel", r#"{"open": true}"#),
    (
        "F6",
        "General",
        "ui.library",
        r#"{"open": true, "tab": "matcaps"}"#,
    ),
    ("F9", "General", "ui.adjust", r#"{"open": true}"#),
    (
        "Ctrl+L",
        "General",
        "ui.palette",
        r#"{"form": "ui.layout", "ask": ["name"]}"#,
    ),
    (
        "Ctrl+Shift+Alt+I",
        "General",
        "ui.palette",
        r#"{"form": "ui.layout", "ask": ["save"]}"#,
    ),
    (
        "Ctrl+1",
        "General",
        "ui.layout",
        r#"{"name": "arrangement-1"}"#,
    ),
    (
        "Ctrl+2",
        "General",
        "ui.layout",
        r#"{"name": "arrangement-2"}"#,
    ),
    (
        "Ctrl+3",
        "General",
        "ui.layout",
        r#"{"name": "arrangement-3"}"#,
    ),
    (
        "Ctrl+Shift+1",
        "General",
        "ui.layout",
        r#"{"save": "arrangement-1"}"#,
    ),
    (
        "Ctrl+Shift+2",
        "General",
        "ui.layout",
        r#"{"save": "arrangement-2"}"#,
    ),
    (
        "Ctrl+Shift+3",
        "General",
        "ui.layout",
        r#"{"save": "arrangement-3"}"#,
    ),
    // interface.default-bindings-view: a view key acts on the view under the pointer.
    ("F", "View", "view.frame", r#"{"view": "main"}"#),
    (
        "Alt+F",
        "View",
        "view.frame",
        r#"{"view": "main", "target": "mask"}"#,
    ),
    (
        "Period",
        "View",
        "view.frame",
        r#"{"view": "main", "target": "last-edit"}"#,
    ),
    (
        "Shift+F",
        "View",
        "view.look",
        r#"{"view": "main", "flip_wireframe": true}"#,
    ),
    (
        "P",
        "View",
        "view.lens",
        r#"{"view": "main", "projection": "toggle"}"#,
    ),
    ("Shift+P", "View", "view.display", r#"{"floor": true}"#),
    ("X", "View", "brush.set", r#"{"flip_mirror": "x"}"#),
    (
        "V X",
        "View",
        "view.align",
        r#"{"view": "main", "to": "left", "toggle": true}"#,
    ),
    (
        "V Y",
        "View",
        "view.align",
        r#"{"view": "main", "to": "top", "toggle": true}"#,
    ),
    (
        "V Z",
        "View",
        "view.align",
        r#"{"view": "main", "to": "front", "toggle": true}"#,
    ),
    (
        "V I",
        "View",
        "view.align",
        r#"{"view": "main", "to": "iso"}"#,
    ),
    ("V H", "View", "view.level", r#"{"view": "main"}"#),
    ("V L", "View", "view.lock", r#"{"view": "main"}"#),
    ("Alt+Z", "View", "view.undo", r#"{"view": "main"}"#),
    ("Shift+Alt+Z", "View", "view.redo", r#"{"view": "main"}"#),
    // interface.default-bindings-sculpt.
    ("B S T", "Sculpt", "brush.set", r#"{"brush": "Draw"}"#),
    ("B C B", "Sculpt", "brush.set", r#"{"brush": "Clay Build"}"#),
    (
        "B S S",
        "Sculpt",
        "brush.set",
        r#"{"brush": "Smooth Strong"}"#,
    ),
    (
        "B M E",
        "Sculpt",
        "brush.set",
        r#"{"brush": "Move Elastic"}"#,
    ),
    (
        "B M P",
        "Sculpt",
        "brush.set",
        r#"{"brush": "Smooth Polish"}"#,
    ),
    ("B C C", "Sculpt", "brush.set", r#"{"brush": "Clip Curve"}"#),
    ("B C L", "Sculpt", "brush.set", r#"{"brush": "Clay Line"}"#),
    ("B M V", "Sculpt", "brush.set", r#"{"brush": "Move"}"#),
    ("B S M", "Sculpt", "brush.set", r#"{"brush": "Smooth"}"#),
    ("B I N", "Sculpt", "brush.set", r#"{"brush": "Inflate"}"#),
    ("B P I", "Sculpt", "brush.set", r#"{"brush": "Pinch"}"#),
    (
        "B D S",
        "Sculpt",
        "brush.set",
        r#"{"brush": "Sharp Crease"}"#,
    ),
    ("B S H", "Sculpt", "brush.set", r#"{"brush": "Pull Out"}"#),
    ("B T D", "Sculpt", "brush.set", r#"{"brush": "Trim"}"#),
    (
        "B H P",
        "Sculpt",
        "brush.set",
        r#"{"brush": "Flat Polish"}"#,
    ),
    ("B C A", "Sculpt", "brush.set", r#"{"brush": "Clay"}"#),
    (
        "B M C",
        "Sculpt",
        "brush.set",
        r#"{"brush": "Move Connected"}"#,
    ),
    (
        "B P N",
        "Sculpt",
        "brush.set",
        r#"{"brush": "Pull Out Along Normal"}"#,
    ),
    ("B M K", "Sculpt", "brush.set", r#"{"brush": "Mask Pen"}"#),
    (
        "B S R",
        "Sculpt",
        "brush.set",
        r#"{"brush": "Select Rect"}"#,
    ),
    ("B U D", "Sculpt", "brush.set", r#"{"brush": "UV Density"}"#),
    ("B U H", "Sculpt", "brush.set", r#"{"brush": "UV Hint"}"#),
    ("B P A", "Sculpt", "brush.set", r#"{"brush": "Paint"}"#),
    (
        "B P B",
        "Sculpt",
        "brush.set",
        r#"{"brush": "Paint Blend"}"#,
    ),
    ("S", "Sculpt", "brush.set", r#"{"size_px": "drag"}"#),
    ("U", "Sculpt", "brush.set", r#"{"strength": "drag"}"#),
    ("O", "Sculpt", "brush.set", r#"{"falloff": "drag"}"#),
    ("[", "Sculpt", "brush.set", r#"{"size_step": -1}"#),
    ("]", "Sculpt", "brush.set", r#"{"size_step": 1}"#),
    (
        "L",
        "Sculpt",
        "brush.set",
        r#"{"settings": {"stroke": {"smoothing": {"radius": 1.0}}}}"#,
    ),
    ("Backtick", "Sculpt", "brush.set", r#"{"previous": true}"#),
    ("Shift+X", "Sculpt", "color.set", r#"{"swap": true}"#),
    ("C", "Sculpt", "color.set", r#"{"pick": "pointer"}"#),
    // interface.default-bindings-mask.
    ("Ctrl+H", "Mask", "view.display", r#"{"mask": false}"#),
    // Invert and Clear take the hidden points too (`R9`).
    ("Ctrl+I", "Mask", "mask.invert", r#"{"scope": "all"}"#),
    ("Ctrl+A", "Mask", "mask.set", r#"{"value": 1}"#),
    (
        "Alt+M",
        "Mask",
        "mask.set",
        r#"{"value": 0, "scope": "all"}"#,
    ),
    (
        "Ctrl+W",
        "Mask",
        "group.make",
        r#"{"maker": {"by": "masked", "clear_mask": true}}"#,
    ),
    ("Ctrl+Shift+X", "Mask", "visibility.grow", "{}"),
    ("Ctrl+Shift+S", "Mask", "visibility.shrink", "{}"),
    (
        "Ctrl+Shift+A",
        "Mask",
        "visibility.grow",
        r#"{"to": "shells"}"#,
    ),
    (
        "Ctrl+Shift+Q",
        "Mask",
        "visibility.grow",
        r#"{"to": "groups"}"#,
    ),
    // interface.default-bindings-geometry.
    ("Ctrl+D", "Geometry", "level.subdivide", "{}"),
    ("D", "Geometry", "level.step", r#"{"by": 1}"#),
    ("Shift+D", "Geometry", "level.step", r#"{"by": -1}"#),
    ("Ctrl+R", "Geometry", "mesh.voxel_remesh", "{}"),
    (
        "Backslash",
        "Geometry",
        "live_detail.set",
        r#"{"toggle": true}"#,
    ),
    ("Ctrl+Shift+D", "Geometry", "part.duplicate", "{}"),
];

/// A key as the keymap writes it (`Ctrl+Shift+Z`): its name and the modifiers held with it.
fn chord(text: &str) -> (&str, Modifiers) {
    let mut parts: Vec<&str> = text.split('+').collect();
    let name = parts.pop().unwrap_or_default();
    let held = Modifiers {
        ctrl: parts.contains(&"Ctrl"),
        shift: parts.contains(&"Shift"),
        alt: parts.contains(&"Alt"),
        logo: false,
    };
    (name, held)
}

/// What pressing `keys` (a key or a sequence) asks the window to run, not run, other than the
/// held keys' brush slot. With `hold`, each key is held past the activation time.
fn asked(ui: &mut Ui, keys: &str, hold: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    for text in keys.split_whitespace() {
        let (name, held) = chord(text);
        for down in [true, false] {
            if hold && !down {
                ui.wait(0.3);
            }
            let mut studio = ui.host.lock();
            studio.input(&[key(name, down, held)], held);
            studio.frame().unwrap_or_else(|e| panic!("{e}"));
            steps.extend(studio.shell.take_commands());
        }
    }
    steps.retain(|s| s.command != "brush.slot");
    steps
}

/// Presses `keys` (a key or a sequence) and runs what they ask for, as the window does.
fn press(ui: &mut Ui, keys: &str) {
    for text in keys.split_whitespace() {
        let (name, held) = chord(text);
        ui.tap(name, held);
    }
}

/// Catalog `interface.default-bindings`: "inject each row's key in its context: each runs the
/// named command". Each bound row, pressed in a new window outside any text field, asks for its
/// command with its arguments; a key held to set a value asks once it is let go.
#[test]
fn each_default_key_asks_for_its_rows_command() {
    let mut ui = Ui::new();
    for (keys, context, command, args) in ROWS {
        let args: Value = serde_json::from_str(args).unwrap_or_else(|e| panic!("{keys}: {e}"));
        let drag = (args.as_object().into_iter().flatten()).find(|(_, v)| *v == "drag");
        let got = asked(&mut ui, keys, drag.is_some());
        // C's pick asks for nothing with the pen off every part (tests/layouts/picker.rs).
        if args.get("pick").is_some() {
            assert!(got.is_empty(), "{keys}: {got:?}");
            continue;
        }
        let first = got.first().map(|s| (s.command.as_str(), s.args.clone()));
        let want = match drag {
            Some((arg, _)) => {
                let set = first.as_ref().and_then(|(_, a)| a.get(arg)).cloned();
                assert!(
                    set.as_ref().is_some_and(Value::is_number),
                    "{keys}: {got:?}"
                );
                json!({ arg.clone(): set })
            }
            None => args,
        };
        assert_eq!(first, Some((*command, want)), "{keys} ({context}): {got:?}");
        assert!(
            got.iter().all(|s| s.command == *command),
            "{keys} asks for nothing else: {got:?}"
        );
    }
}

/// Every default key is a row of the catalog's tables, and every bound row is a default key.
#[test]
fn the_default_keys_are_the_bound_rows() {
    let mut defaults: Vec<String> = (Keymap::default().bindings.iter())
        .map(|b| b.key.clone())
        .collect();
    let mut rows: Vec<String> = ROWS.iter().map(|r| r.0.to_string()).collect();
    defaults.sort();
    rows.sort();
    assert_eq!(defaults, rows);
}

/// Catalog `interface.default-bindings`: "the generated cheat sheet has one line per row". The
/// keys sheet lists every key once, under its context's heading.
#[test]
fn the_keys_sheet_lists_every_key_once_under_its_context() {
    let mut ui = Ui::new();
    // Tall enough for the whole sheet: `ui.describe` lists only what shows.
    ui.host.lock().resize([1800, 6000], 1.0);
    ui.ok("ui.settings", json!({ "open": true, "page": "Keys" }));
    ui.settle();
    let contexts = ["General", "View", "Sculpt", "Mask", "Geometry"];
    let mut listed: Vec<(&str, Vec<String>)> = contexts.map(|c| (c, Vec::new())).to_vec();
    let mut under = None;
    for label in ui.labels("settings") {
        if let Some(at) = contexts.iter().position(|c| label == format!("{c} keys")) {
            under = Some(at);
        } else if let (Some(at), Some((key, _))) = (under, label.split_once(": ")) {
            listed[at].1.push(key.to_string());
        }
    }
    for (context, mut keys) in listed {
        let mut rows: Vec<String> = (ROWS.iter())
            .filter(|r| r.1 == context)
            .map(|r| r.0.to_string())
            .collect();
        keys.sort();
        rows.sort();
        assert_eq!(keys, rows, "{context}");
    }
}

/// Catalog `interface.default-bindings`: "Tab hide or show all panels". egui hands the keyboard
/// to the first widget on Tab; the window takes it back, so the keys go on working after.
#[test]
fn tab_hides_and_shows_the_panels_and_the_keys_go_on_working() {
    let mut ui = Ui::new();
    let none = Modifiers::default();
    let chrome = |ui: &mut Ui| ui.state()["layout.chrome"].clone();
    assert_eq!(chrome(&mut ui), json!(true), "panels showing");
    ui.tap("Tab", none);
    assert_eq!(chrome(&mut ui), json!(false), "Tab hides them");
    ui.tap("Tab", none);
    assert_eq!(chrome(&mut ui), json!(true), "and shows them");
    ui.tap("X", none);
    assert_eq!(ui.state()["symmetry.x"], json!(true), "X still runs");
    ui.tap("Space", none);
    assert!(ui.host.lock().shell.palette.open, "Space opens the palette");
}

/// Column `name` of `mesh`, one number per face or point, or none.
fn column(mesh: &PlainMesh, name: &str) -> Vec<f64> {
    match mesh.attributes.get(name).map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.iter().map(|&x| f64::from(x)).collect(),
        Some(AttrValues::F32(v)) => v.iter().map(|&x| f64::from(x)).collect(),
        None => Vec::new(),
    }
}

/// The view `main`'s camera as JSON text (equal text is equal bits).
fn camera(ui: &mut Ui) -> String {
    ui.ok("view.get", json!({ "view": "main" }))["camera"].to_string()
}

/// Catalog `interface.default-bindings-view`, its Check: "F frames, P toggles perspective,
/// Shift+P toggles the floor grid, V then Z turns to the front view and again to the back, and
/// with no canvas stencil Ctrl+H shows the mask".
#[test]
fn the_view_keys_frame_toggle_and_turn_to_the_front_and_back() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    ui.ok(
        "view.orbit",
        json!({ "view": "main", "right_deg": 40.0, "up_deg": 20.0 }),
    );
    ui.ok("view.dolly", json!({ "view": "main", "factor": 3.0 }));
    let moved = camera(&mut ui);
    press(&mut ui, "F");
    let framed = camera(&mut ui);
    assert_ne!(framed, moved, "F moves the camera");
    ui.ok("view.frame", json!({ "view": "main" }));
    assert_eq!(camera(&mut ui), framed, "to where view.frame puts it");
    for (keys, value) in [
        ("P", "view.projection"),
        ("Shift+P", "view.floor"),
        ("Ctrl+H", "view.mask"),
    ] {
        let before = ui.state()[value].clone();
        press(&mut ui, keys);
        assert_ne!(ui.state()[value], before, "{keys} turns {value}");
        press(&mut ui, keys);
        assert_eq!(ui.state()[value], before, "{keys} again turns it back");
    }
    for side in ["front", "back"] {
        press(&mut ui, "V Z");
        let turned = camera(&mut ui);
        ui.ok("view.align", json!({ "view": "main", "to": side }));
        assert_eq!(camera(&mut ui), turned, "V Z turned to the {side}");
    }
}

/// Catalog `interface.default-bindings-mask`, its Check: "Ctrl+A masks every point, Ctrl+I
/// inverts the mask and Ctrl+H toggles its display; Ctrl+W turns the masked faces into one new
/// group and leaves no mask; with part of the mesh hidden Shift+Ctrl+Q makes every group with a
/// visible face fully visible". Ctrl+H is the view Check's.
#[test]
fn the_mask_keys_mask_invert_group_and_grow_to_groups() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let mask = |ui: &mut Ui| column(&ui.session.mesh("sphere"), "mask");
    press(&mut ui, "Ctrl+A");
    let one = |x: &f64| (x - 1.0).abs() < 1e-6;
    let masked = mask(&mut ui);
    assert!(
        !masked.is_empty() && masked.iter().all(one),
        "Ctrl+A masks every point"
    );
    press(&mut ui, "Ctrl+I");
    assert!(mask(&mut ui).iter().all(|&m| m == 0.0), "Ctrl+I inverts it");
    let old = column(&ui.session.mesh("sphere"), "group");
    press(&mut ui, "Ctrl+A");
    press(&mut ui, "Ctrl+W");
    let groups = column(&ui.session.mesh("sphere"), "group");
    assert!(
        !groups.is_empty() && groups.iter().all(one),
        "one group: {groups:?}"
    );
    assert!(!old.iter().any(one), "a new one: {old:?}");
    assert!(mask(&mut ui).iter().all(|&m| m == 0.0), "and no mask");
    // Two groups, and two faces of the first showing.
    let faces = groups.len();
    let first: Vec<usize> = (0..faces / 2).collect();
    let select = json!({ "select": { "by": "faces", "indexes": first }, "id": 2 });
    ui.ok("group.assign", select);
    let select = json!({ "select": { "by": "faces", "indexes": [0, 1] } });
    ui.ok("visibility.only", select);
    press(&mut ui, "Ctrl+Shift+Q");
    let hide = column(&ui.session.mesh("sphere"), "hide");
    let shown: Vec<usize> = (0..faces).filter(|&f| hide[f] == 0.0).collect();
    assert_eq!(shown, first, "every face of the group with a visible face");
}

/// Decision 0060, section 3 as amended for H3: each key the Mesh, Mask, Visible and Groups
/// menus show at a row's right, pressed with the menu shut, asks for that row's command.
#[test]
fn each_key_a_menu_shows_asks_for_its_rows_command() {
    let mut ui = Ui::new();
    for (title, label, keys) in [
        ("Mesh", "Voxel Remesh", "Ctrl+R"),
        ("Mask", "All", "Ctrl+A"),
        ("Mask", "Invert", "Ctrl+I"),
        ("Mask", "Clear", "Alt+M"),
        ("Visible", "Grow", "Ctrl+Shift+X"),
        ("Visible", "Shrink", "Ctrl+Shift+S"),
        ("Visible", "Shells", "Ctrl+Shift+A"),
        ("Visible", "Groups", "Ctrl+Shift+Q"),
        ("Groups", "Mask and clear it", "Ctrl+W"),
    ] {
        crate::header::open(&mut ui, title);
        let row = crate::header::item(&mut ui, label);
        assert_eq!(row["key"], keys, "{title} > {label}: {row}");
        let step: Step = serde_json::from_value(row["command"].clone())
            .unwrap_or_else(|e| panic!("{title} > {label}: {e}: {row}"));
        ui.ok("ui.menu", json!({ "open": false }));
        ui.settle();
        assert_eq!(asked(&mut ui, keys, false), [step], "{keys}");
    }
}

/// Decision 0060, section 3 as amended for H4 (items 2.1 and 10): every key the Add, Part, View
/// and Window menus show, a line's buttons' included, pressed with the menu shut asks for that
/// row's command; a view key asks for it on the view under the pointer, where the row runs on
/// the 3D view.
#[test]
fn each_key_the_add_part_view_and_window_menus_show_asks_for_its_rows_command() {
    let mut ui = Ui::new();
    let mut keyed = Vec::new();
    for title in ["Add", "Part", "View", "Window"] {
        crate::header::open(&mut ui, title);
        for row in crate::header::items(&mut ui) {
            let Some(keys) = row["key"].as_str() else {
                continue;
            };
            let step: Step = serde_json::from_value(row["command"].clone())
                .unwrap_or_else(|e| panic!("{title}: {e}: {row}"));
            keyed.push((keys.to_string(), step));
        }
        ui.ok("ui.menu", json!({ "open": false }));
        ui.settle();
    }
    let named: Vec<&str> = keyed.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        named,
        [
            "Ctrl+Shift+D",
            "V H",
            "V X",
            "V Y",
            "V Z",
            "V I",
            "Alt+Z",
            "Shift+Alt+Z",
            "Ctrl+1",
            "Ctrl+2",
            "Ctrl+3",
            "Ctrl+Shift+1",
            "Ctrl+Shift+2",
            "Ctrl+Shift+3",
            "Tab",
        ]
    );
    for (keys, step) in keyed {
        let mut got = asked(&mut ui, &keys, false);
        for s in &mut got {
            if let Some(args) = s
                .args
                .as_object_mut()
                .filter(|_| step.args["view"].is_null())
            {
                args.remove("view");
            }
        }
        assert_eq!(got, [step], "{keys}");
    }
}

/// Catalog `interface.default-bindings-geometry`, its Check: "D at level 1 of a 3-level Part
/// goes to level 2 and Shift+D back to 1; Backslash flips the Live Detail state; Shift+Ctrl+D
/// adds one Part".
#[test]
fn the_geometry_keys_step_levels_flip_live_detail_and_duplicate() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 2 }));
    let on = |ui: &mut Ui| ui.state()["live_detail.on"].clone();
    let before = on(&mut ui);
    press(&mut ui, "Backslash");
    assert_ne!(on(&mut ui), before, "Backslash flips Live Detail");
    press(&mut ui, "Backslash");
    assert_eq!(on(&mut ui), before, "and back");
    let parts = |ui: &mut Ui| {
        ui.ok("mesh.stats", json!({}))["parts"]
            .as_array()
            .map(Vec::len)
    };
    press(&mut ui, "Ctrl+Shift+D");
    assert_eq!(parts(&mut ui), Some(2), "Shift+Ctrl+D adds one part");
    press(&mut ui, "Ctrl+D");
    press(&mut ui, "Ctrl+D");
    let level = |ui: &mut Ui| {
        let levels = ui.ok("level.describe", json!({}));
        (levels["count"].clone(), levels["active"].clone())
    };
    assert_eq!(
        level(&mut ui),
        (json!(3), json!(3)),
        "Ctrl+D twice: three levels"
    );
    ui.ok("level.set", json!({ "level": 1 }));
    press(&mut ui, "D");
    assert_eq!(level(&mut ui).1, json!(2), "D goes up a level");
    press(&mut ui, "Shift+D");
    assert_eq!(level(&mut ui).1, json!(1), "Shift+D goes back");
}
