//! The palette is the back door to everything: it finds each control a panel shows and says
//! where it lives, lists what was picked last first, and asks for the arguments
//! a command needs (a file to import) before running it.

use gs_platform::{Input, Modifiers};
use gs_protocol::{ErrorCode, Step};
use serde_json::{Value, json};

use crate::rig::{Ui, key};

/// The palette's rows for `query`, as `ui.palette` reports them.
fn rows(ui: &mut Ui, query: &str) -> Vec<Value> {
    let info = ui.ok("ui.palette", json!({ "query": query }));
    info["rows"].as_array().cloned().unwrap_or_default()
}

#[test]
fn search_finds_every_control_and_says_where_it_lives() {
    let mut ui = Ui::new();
    // With the gizmo tool on, the brush's section shows the gizmo's settings too; File open,
    // its items show, whose home the palette names as Header > File (decision 0060, section 1).
    ui.ok("ui.gizmo", json!({ "tool": true }));
    ui.ok("ui.menu", json!({ "open": "file" }));
    let (controls, brushes) = {
        let directions = &ui.host.lock().shell.directions;
        (directions.controls.clone(), directions.brush_controls())
    };
    {
        let d = ui.describe();
        let shown = (d["panels"].as_array().into_iter().flatten()).filter(|p| {
            let id = p["id"].as_str().unwrap_or_default();
            // The orientation figure's discs are drawn in code, not controls, and run what Header
            // > View's Look along runs.
            !["status", "viewport", "palette", "wheel", "figure"].contains(&id)
        });
        let mut menus = 0;
        for panel in shown.collect::<Vec<_>>() {
            let title = panel["title"].as_str().unwrap_or_default();
            let title = if panel["id"] == "menu" {
                menus += 1;
                format!("Header > {title}")
            } else {
                title.to_string()
            };
            for w in panel["widgets"].as_array().into_iter().flatten() {
                let Ok(step) = serde_json::from_value::<Step>(w["command"].clone()) else {
                    continue;
                };
                let all = (controls.sets.iter().flat_map(|s| &s.controls)).chain(&brushes);
                let Some(label) = all
                    .into_iter()
                    .find(|c| c.step() == Some(step.clone()))
                    .map(|c| c.label.clone())
                else {
                    continue;
                };
                let found = rows(&mut ui, &label)
                    .into_iter()
                    .find(|r| r["label"] == label.as_str());
                let places = found.map(|r| r["places"].clone()).unwrap_or_default();
                let lives = places
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|p| p.as_str().is_some_and(|p| p.starts_with(&title)));
                assert!(lives, "{label} lives in {title}: {places}");
            }
        }
        assert_eq!(menus, 1, "File's menu was among them");
    }
    ui.ok("ui.menu", json!({ "open": false }));
    // A command whose summary only mentions the query comes after the controls it names.
    let found = rows(&mut ui, "clay");
    let at = |label: &str| found.iter().position(|r| r["label"] == label);
    let (brush, light) = (at("Clay"), at("light.set"));
    assert!(brush.is_some() && light.is_some(), "both found: {found:?}");
    assert!(brush < light, "the Clay brush before light.set: {found:?}");
    ui.ok("ui.palette", json!({ "open": false }));
}

/// The controls that left the scene tray for the header's menus (decision 0060, section 3 as
/// amended for H3) by their full labels, under the home the palette names.
const MOVED: [(&str, &[&str]); 11] = [
    (
        "Header > Mesh > Remesh",
        &["Voxel Remesh", "Voxel mode", "Quad Remesh", "Decimate"],
    ),
    (
        "Header > Mesh > Modify",
        &[
            "Mirror and weld X",
            "Mirror and weld Y",
            "Mirror and weld Z",
            "Project Detail",
        ],
    ),
    ("Header > Mask", &["Mask all", "Invert mask", "Clear mask"]),
    (
        "Header > Mask > Filter",
        &[
            "Grow mask",
            "Shrink mask",
            "Blur mask",
            "Sharpen mask",
            "Boost mask",
            "Dilute mask",
            "Normalize mask",
        ],
    ),
    (
        "Header > Mask > Make",
        &["Mask by cavity", "Mask by features"],
    ),
    ("Header > Visible > Hide", &["Hide masked", "Hide unmasked"]),
    ("Header > Visible > Show", &["Show unmasked", "Show all"]),
    (
        "Header > Visible > Area",
        &[
            "Grow visible area",
            "Shrink visible area",
            "Invert visibility",
        ],
    ),
    (
        "Header > Visible > Whole",
        &["Show whole shells", "Show whole groups"],
    ),
    (
        "Header > Groups > Make from",
        &[
            "Groups from shells",
            "Groups by surface angle",
            "Group visible faces",
            "Group faces facing the camera",
            "Group masked faces",
            "Group masked faces and clear mask",
        ],
    ),
    (
        "Header > Groups > Edit",
        &["Visible faces to a group", "Merge groups"],
    ),
];

/// The controls of the Add, Part, View and Window menus (decision 0060, section 3 as amended for
/// H4) by their full labels, under the home the palette names: those that left the scene tray,
/// and View's rows for what only keys ran.
const MOVED_H4: [(&str, &[&str]); 12] = [
    ("Header > Add", &["Sphere", "Cube", "Cylinder"]),
    (
        "Header > Part",
        &[
            "Duplicate",
            "Mirror copy X",
            "Mirror copy Y",
            "Mirror copy Z",
        ],
    ),
    (
        "Header > Part > Order",
        &["Rename", "Move up", "Move down", "Show all parts"],
    ),
    (
        "Header > Part > Split",
        &[
            "Split into shells",
            "Split by groups",
            "Split by mask",
            "Split off hidden",
            "Extract",
        ],
    ),
    (
        "Header > Part > Merge",
        &["Merge down", "Merge visible", "Remesh visible"],
    ),
    (
        "Header > Part > Delete",
        &["Delete", "Delete others", "Delete hidden", "Delete all"],
    ),
    (
        "Header > View",
        &[
            "Level",
            "Look along X",
            "Look along Y",
            "Look along Z",
            "Iso view",
        ],
    ),
    (
        "Header > View > Camera",
        &["Undo view move", "Redo view move"],
    ),
    (
        "Header > View > Reference",
        &[
            "Reference from a file",
            "Reference: this scene",
            "Reference from parts",
        ],
    ),
    ("Header > Window", &["Settings", "Left hand", "Right hand"]),
    (
        "Header > Window > Arrangement",
        &[
            "Arrangement 1",
            "Arrangement 2",
            "Arrangement 3",
            "Save arrangement 1",
            "Save arrangement 2",
            "Save arrangement 3",
            "Next layout",
            "Previous layout",
            "Restore layout",
        ],
    ),
    (
        "Header > Window > Panels",
        &["Left tray", "Right tray", "All panels", "One section open"],
    ),
];

/// Decision 0060, section 3 as amended for H3: the search finds each control that left the
/// scene tray for the Mesh, Mask, Visible and Groups menus by its full label, under its menu and
/// group, and no longer in the tray.
#[test]
fn search_finds_each_moved_control_under_its_menu() {
    each_under_its_home(&MOVED);
}

/// Decision 0060, section 3 as amended for H4: the search finds each control of the Add, Part,
/// View and Window menus by its full label, under its menu and group, and not in the tray.
#[test]
fn search_finds_each_control_of_add_part_view_and_window_under_its_menu() {
    each_under_its_home(&MOVED_H4);
}

/// The search finds each of `moved`'s labels under its home and not in the scene tray.
fn each_under_its_home(moved: &[(&str, &[&str])]) {
    let mut ui = Ui::new();
    for &(home, labels) in moved {
        for &label in labels {
            let found = rows(&mut ui, label)
                .into_iter()
                .find(|r| r["label"] == label);
            let places = found.map(|r| r["places"].clone()).unwrap_or_default();
            let places: Vec<&str> = (places.as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .collect();
            assert!(
                places.contains(&home),
                "{label} lives in {home}: {places:?}"
            );
            let tray = places.iter().find(|p| p.starts_with("Scene tray"));
            assert_eq!(tray, None, "{label} left the scene tray: {places:?}");
        }
    }
    ui.ok("ui.palette", json!({ "open": false }));
}

#[test]
fn recent_picks_come_first_and_a_command_asks_for_its_arguments() {
    let mut ui = Ui::new();
    let none = Modifiers::default();
    ui.ok("ui.palette", json!({ "query": "clay build" }));
    ui.input(&[key("Enter", true, none)], none);
    assert_eq!(
        ui.state()["brush"],
        json!("Clay Build"),
        "picked from the palette"
    );
    let first = rows(&mut ui, "").first().map(|r| r["label"].clone());
    assert_eq!(
        first,
        Some(json!("Clay Build")),
        "the last pick comes first"
    );
    ui.ok("ui.palette", json!({ "open": false }));
    // Import: the Workbench's File > Import… opens the form, which asks for the path.
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(format!("palette-import-{}.obj", std::process::id()));
    let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 3 2\nf 1 2 4\nf 1 4 3\nf 2 3 4\n";
    std::fs::write(&path, obj).unwrap_or_else(|e| panic!("{e}"));
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    ui.ok("ui.menu", json!({ "open": "file" }));
    let import = ui
        .widgets("menu")
        .into_iter()
        .find(|w| w["label"] == "Import…")
        .unwrap_or_default();
    ui.ok("ui.menu", json!({ "open": false }));
    let step: Step =
        serde_json::from_value(import["command"].clone()).unwrap_or_else(|e| panic!("{e}"));
    let asked = ui.ok(&step.command, step.args);
    assert_eq!(asked["form"], json!("file.import"), "{asked}");
    ui.input(&[], none);
    let fields = ui.labels("palette");
    assert!(
        fields.contains(&"Path".to_string()),
        "a field for the path: {fields:?}"
    );
    ui.input(&[Input::Text(path.display().to_string())], none);
    ui.input(&[key("Enter", true, none)], none);
    let stats = ui.ok("mesh.stats", json!({}));
    assert_eq!(
        stats["parts"].as_array().map(Vec::len),
        Some(1),
        "imported: {stats}"
    );
    assert!(
        !ui.ok("ui.palette", json!({ "open": false }))["open"]
            .as_bool()
            .unwrap_or(true)
    );
}

/// A form can start with some arguments given (`args`): it asks only for the rest, makes
/// the same call as choosing them all and leaves out what nothing chose. What a control gives
/// or asks for is refused on a dry run, which the window makes for every control it shows, so
/// a mistake in a layout file fails the controls test instead of waiting for a click.
#[test]
fn a_form_with_arguments_given_asks_only_for_the_rest() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    // The arguments the form asks, and the call its Run button makes once `chosen` is chosen.
    let form = |ui: &mut Ui, palette: Value, chosen: &[&str]| {
        ui.ok("ui.palette", palette);
        let mut fields = Vec::new();
        if let Some(form) = ui.host.lock().shell.palette.form.as_mut() {
            fields = form.fields.iter().map(|(name, _)| json!(name)).collect();
            for ((name, _), value) in form.fields.iter().zip(chosen) {
                form.chosen.insert(name.clone(), json!(value));
            }
        }
        ui.input(&[], Modifiers::default());
        let widgets = ui.widgets("palette");
        let run = widgets.iter().find(|w| w["label"] == "Split part");
        let run = run.map(|w| w["command"].clone()).unwrap_or_default();
        ui.ok("ui.palette", json!({ "open": false }));
        (fields, run)
    };
    let given = json!({ "form": "part.split", "args": { "by": "shells" }, "ask": ["part"] });
    let (fields, run) = form(&mut ui, given, &["sphere"]);
    assert_eq!(fields, [json!("part")], "only the part is asked for");
    let typed = json!({ "form": "part.split", "ask": ["part"] });
    let (fields, both) = form(&mut ui, typed, &["shells", "sphere"]);
    assert_eq!(fields, [json!("by"), json!("part")]);
    assert_eq!(run, both, "the same call as choosing both");
    assert_eq!(run["args"], json!({ "by": "shells", "part": "sphere" }));
    let given = json!({ "form": "part.split", "args": { "by": "shells" }, "ask": ["part"] });
    let (_, only) = form(&mut ui, given, &[]);
    assert_eq!(
        only["args"],
        json!({ "by": "shells" }),
        "a part nothing chose is left out: the only part"
    );
    for wrong in [
        json!({ "form": "part.split", "args": { "by": "shels" } }),
        json!({ "form": "part.split", "args": { "bye": "shells" } }),
        json!({ "form": "part.split", "args": { "by": "shells" }, "ask": ["by"] }),
        json!({ "form": "part.split", "ask": ["prat"] }),
        json!({ "form": "part.splits" }),
        json!({ "args": { "by": "shells" } }),
    ] {
        let refused = ui.dry_run("ui.palette", wrong.clone());
        assert_eq!(refused, Some(ErrorCode::ArgsInvalid), "{wrong}");
    }
}

#[test]
fn the_scene_keys_run_what_the_scene_buttons_run_and_the_buttons_show_them() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    let ctrl_alt = Modifiers { alt: true, ..ctrl };
    // New and Revert open the form their buttons open, so a key alone never drops work.
    for (k, held, form) in [("N", ctrl, "file.new"), ("R", ctrl_alt, "file.revert")] {
        ui.tap(k, held);
        let open = ui.ok("ui.palette", json!({}));
        assert_eq!(open["form"], json!(form), "{k}: {open}");
        ui.ok("ui.palette", json!({ "open": false }));
        // A frame lets go of the form's field, as the window's next frame does.
        ui.settle();
    }
    // Ctrl and Alt let go: held, a press on a button aims at it for a key (bind_click.rs).
    ui.input(&[], Modifiers::default());
    // The scene's buttons are the header's File items (decision 0060, section 3).
    ui.press("header > File");
    let items = ui.widgets("menu");
    let labels = ["New…", "Revert…", "Save next", "Quick save"];
    let shown = labels.map(|label| {
        let button = items.iter().find(|w| w["label"] == label);
        button.map(|b| b["key"].clone()).unwrap_or_default()
    });
    assert_eq!(
        shown,
        ["Ctrl+N", "Ctrl+Alt+R", "Ctrl+Alt+S", "9"].map(|k| json!(k)),
        "each item shows its key"
    );
}

/// Ctrl+H shows and hides the mask's darkening and Shift+P the floor grid (decision 0017,
/// item 11): each runs what the Show mask and Floor switches run when tapped, both ways, and
/// the switches and their palette rows show the keys.
#[test]
fn ctrl_h_and_shift_p_run_what_show_mask_and_floor_run_and_both_show_the_keys() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    ui.open_look();
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    // Each switch as drawn, and as the palette names it: the mask's is Mask on the Look
    // options' Show row (decision 0060 as amended for R1).
    let switches = [
        ("H", ctrl, "Ctrl+H", "look", ["Mask", "Show mask"], "mask"),
        ("P", shift, "Shift+P", "header", ["Floor", "Floor"], "floor"),
    ];
    for (k, held, chord, panel, [label, named], arg) in switches {
        let shows = format!("view.{arg}");
        let start = ui.state()[&shows].clone();
        for _ in 0..2 {
            let switch = (ui.widgets(panel).into_iter()).find(|w| w["label"] == label);
            let switch = switch.unwrap_or_else(|| panic!("{label} shows in {panel}"));
            assert_eq!(switch["key"], json!(chord), "{label} shows its key");
            let tapped = &switch["command"];
            assert_eq!(tapped["command"], "view.display", "{label}");
            let before = ui.logged("view.display").len();
            ui.tap(k, held);
            assert_eq!(
                ui.logged("view.display").len(),
                before + 1,
                "{chord} ran view.display once"
            );
            let now = ui.state()[&shows].clone();
            assert!(now.is_boolean(), "{shows} is on or off: {now}");
            assert_eq!(
                now, tapped["args"][arg],
                "{chord} did what a tap on {label} does"
            );
        }
        assert_eq!(ui.state()[&shows], start, "{chord} twice is back as it was");
        let row = rows(&mut ui, named)
            .into_iter()
            .find(|r| r["label"] == named);
        let row = row.unwrap_or_else(|| panic!("the palette finds {named}"));
        assert_eq!(
            row["key"],
            json!(chord),
            "{named}'s palette row shows the key"
        );
        ui.ok("ui.palette", json!({ "open": false }));
        ui.settle();
    }
}
