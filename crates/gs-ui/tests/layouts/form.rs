//! The palette's form draws each argument by its kind (decision 0025, rule 13): a choice is
//! never typed. An enum is a row of lit buttons, with its default lit under More when it has
//! one; a boolean is a switch, and a guard is a plain question with two buttons; a bounded
//! number is a slider held to its ends; a part is a pick of the scene's parts. A path keeps
//! its field, starting in its folder, and over the whole catalog only a path, free text or an
//! unbounded number is typed. Run reads the command's title.

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, folder};
use crate::scene::parts;

/// The palette's widgets whose role is `role`.
fn with_role(ui: &mut Ui, role: &str) -> Vec<Value> {
    let widgets = ui.widgets("palette").into_iter();
    widgets.filter(|w| w["role"] == role).collect()
}

/// The step the form's Run button, labelled with the command's title, sends now.
fn run_step(ui: &mut Ui, title: &str) -> Value {
    ui.widget("palette", title)["command"].clone()
}

/// Whether the palette's form is open.
fn form_open(ui: &Ui) -> bool {
    ui.host.lock().shell.palette.form.is_some()
}

/// Mirror and weld's axis is three buttons and nothing to type; a tap on Y chooses it and runs
/// nothing, and Run, which reads the command's title, sends it as one step.
#[test]
fn mirror_and_welds_axis_is_three_buttons_and_run_sends_the_one_chosen() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.ok("ui.palette", json!({ "form": "mesh.symmetrize" }));
    for axis in ["X", "Y", "Z"] {
        assert_eq!(ui.widget("palette", axis)["role"], "button", "{axis}");
    }
    let typed = with_role(&mut ui, "text field");
    assert!(typed.is_empty(), "nothing to type: {typed:?}");
    ui.press("palette > Y");
    assert!(
        ui.logged("mesh.symmetrize").is_empty(),
        "a choice runs nothing"
    );
    assert_eq!(ui.widget("palette", "Y")["selected"], true, "Y is lit");
    assert_eq!(ui.widget("palette", "X")["selected"], false);
    let step = json!({ "command": "mesh.symmetrize", "args": { "axis": "y" } });
    assert_eq!(run_step(&mut ui, "Mirror and weld"), step);
    ui.press("palette > Mirror and weld");
    assert_eq!(ui.logged("mesh.symmetrize").len(), 1, "one step");
    assert!(!form_open(&ui), "Run closes the form");
}

/// `side` has a default, so it shows under More, shut at first, with that default lit, in
/// words that name the axis chosen; a tap on the other side sends it.
#[test]
fn side_shows_under_more_with_its_default_lit() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.ok("ui.palette", json!({ "form": "mesh.symmetrize" }));
    ui.press("palette > Y");
    let shut = ui.labels("palette");
    let sides = shut.iter().filter(|l| l.starts_with("Keep"));
    assert_eq!(sides.count(), 0, "More is shut at first: {shut:?}");
    ui.press("palette > more");
    assert_eq!(
        ui.widget("palette", "Keep +Y side")["selected"],
        true,
        "the default is lit"
    );
    assert_eq!(ui.widget("palette", "Keep -Y side")["selected"], false);
    ui.press("palette > Keep -Y side");
    let args = json!({ "axis": "y", "side": "negative" });
    assert_eq!(run_step(&mut ui, "Mirror and weld")["args"], args);
}

/// A boolean is a switch: Quad Remesh's Keep groups shows off, its default, and a tap turns it
/// on for Run.
#[test]
fn a_boolean_argument_is_a_switch() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let form = json!({ "form": "mesh.quad_remesh", "ask": ["keep_groups"] });
    ui.ok("ui.palette", form);
    let switch = ui.widget("palette", "Keep groups");
    assert_eq!(switch["role"], "toggle", "{switch}");
    assert_eq!(switch["selected"], false, "off, its default");
    ui.press("palette > Keep groups");
    assert_eq!(ui.widget("palette", "Keep groups")["selected"], true);
    let args = json!({ "keep_groups": true });
    assert_eq!(run_step(&mut ui, "Quad remesh")["args"], args);
}

/// Open with unsaved changes asks a plain question first, naming the scene, with nothing to
/// type: Save, Don't save or Cancel; Don't save opens the file with `discard_changes` (review
/// 0009, C8).
#[test]
fn open_with_unsaved_changes_asks_save_dont_save_or_cancel() {
    let mut ui = Ui::new();
    let path = folder("form-question").join("one.gss");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": path }));
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    let form = json!({ "form": "file.open", "args": { "path": path } });
    ui.ok("ui.palette", form);
    let line = "Save the changes to one.gss before opening another scene?";
    assert_eq!(ui.labels("palette"), [line, "Save", "Don't save", "Cancel"]);
    let typed = with_role(&mut ui, "text field");
    assert!(typed.is_empty(), "nothing to type: {typed:?}");
    let no = ui.widget("palette", "Don't save")["command"].clone();
    let path = path.display().to_string();
    let args = json!({ "path": path, "discard_changes": true });
    assert_eq!(no, json!({ "command": "file.open", "args": args }));
    assert_eq!(parts(&mut ui), 2, "nothing is dropped while it asks");
    ui.press("palette > Don't save");
    assert_eq!(parts(&mut ui), 1, "opened once answered");
    assert!(!form_open(&ui));
}

/// A bounded number is a slider: Voxel Remesh's resolution, 16 to 2048, takes a typed value
/// held to those ends, and the Enter that sets a typed value runs nothing.
#[test]
fn a_bounded_number_is_a_slider_held_to_its_bounds() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let form = json!({ "form": "mesh.voxel_remesh", "ask": ["resolution"] });
    ui.ok("ui.palette", form);
    assert_eq!(ui.widget("palette", "Resolution")["role"], "slider");
    let none = Modifiers::default();
    for (typed, held) in [("5000", 2048), ("3", 16), ("300", 300)] {
        let r = ui.rect("palette > Resolution");
        ui.tap_at(r[2] - 4.0, middle(r)[1]);
        ui.input(&[Input::Text(typed.into())], none);
        ui.tap("Return", none);
        let args = run_step(&mut ui, "Voxel remesh")["args"].clone();
        assert_eq!(args, json!({ "resolution": held }), "{typed} typed");
    }
    assert!(ui.logged("mesh.voxel_remesh").is_empty(), "nothing ran");
}

/// Three numbers its schema marks a color (`x-rgb`), as `light.set`'s color is, are three fields
/// R, G and B; a direction's are X, Y and Z (decision 0025, rule 13, amended for UI-finish).
#[test]
fn light_sets_color_is_three_fields_r_g_and_b() {
    let mut ui = Ui::new();
    let form = json!({ "form": "light.set", "ask": ["color", "direction"] });
    ui.ok("ui.palette", form);
    for label in [
        "Color R",
        "Color G",
        "Color B",
        "Direction X",
        "Direction Z",
    ] {
        assert_eq!(ui.widget("palette", label)["role"], "text field", "{label}");
    }
    let labels = ui.labels("palette");
    assert!(!labels.iter().any(|l| l == "Color X"), "{labels:?}");
}

/// A part argument lists the scene's parts by name with the active one lit, where the engine
/// takes the active part when it is left out; a tap picks another for Run.
#[test]
fn a_part_argument_lists_the_scenes_parts_with_the_active_one_lit() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "name": "head" }));
    ui.ok(
        "primitive.cube",
        json!({ "name": "box", "center": [0, 0, 3] }),
    );
    let form = json!({ "form": "part.mirror_copy", "ask": ["part"] });
    ui.ok("ui.palette", form);
    let lit = |ui: &mut Ui, name: &str| ui.widget("palette", name)["selected"].clone();
    assert_eq!(lit(&mut ui, "box"), true, "the new part is active");
    assert_eq!(lit(&mut ui, "head"), false);
    ui.press("palette > head");
    assert_eq!(
        (lit(&mut ui, "head"), lit(&mut ui, "box")),
        (json!(true), json!(false))
    );
    let args = json!({ "part": "head" });
    assert_eq!(run_step(&mut ui, "Mirror copy")["args"], args);
}

/// Save's path is still a text field, a value, labelled in words and starting in the scene's
/// folder.
#[test]
fn saves_path_is_still_a_text_field_in_its_folder() {
    let mut ui = Ui::new();
    let dir = folder("form-path");
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.ok("file.save", json!({ "path": dir.join("one.gss") }));
    ui.ok(
        "ui.palette",
        json!({ "form": "file.save", "ask": ["path"] }),
    );
    let field = ui.widget("palette", "Path");
    assert_eq!(field["role"], "text field", "{field}");
    let form = ui.host.lock().shell.palette.form.clone();
    let start = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
    let fields = form.map(|f| f.fields).unwrap_or_default();
    assert_eq!(
        fields,
        [("path".to_string(), start)],
        "in the scene's folder"
    );
}

/// Whether `schema` (an argument's, with the command's `defs`) is a choice: words only, a
/// boolean, a number with both ends, or a part.
fn choice(schema: &Value, defs: &Value) -> bool {
    let schema = resolve(schema, defs);
    let alternatives = schema["anyOf"].as_array().or(schema["oneOf"].as_array());
    if let Some(all) = alternatives {
        let named = |s: &Value| resolve(s, defs)["type"] != "null";
        let rest: Vec<&Value> = all.iter().filter(|s| named(s)).collect();
        if rest.len() < all.len() && rest.len() == 1 {
            return choice(rest[0], defs);
        }
        let word = |s: &Value| {
            let s = resolve(s, defs);
            s["const"].is_string() || s["enum"].is_array()
        };
        return part(schema) || all.iter().all(word);
    }
    let typed = |t: &str| {
        schema["type"] == t
            || schema["type"]
                .as_array()
                .is_some_and(|a| a.contains(&json!(t)))
    };
    let bounded = schema["minimum"].is_number() && schema["maximum"].is_number();
    part(schema)
        || schema["enum"].is_array()
        || typed("boolean")
        || ((typed("number") || typed("integer")) && bounded)
}

/// Whether `schema` names a part (the `PartRef` and `Parts` definitions).
fn part(schema: &Value) -> bool {
    let text = schema.to_string();
    text.contains("#/$defs/PartRef") || text.contains("#/$defs/Parts\"")
}

/// `schema` with its `$ref` followed into `defs`.
fn resolve<'a>(schema: &'a Value, defs: &'a Value) -> &'a Value {
    match schema["$ref"].as_str().and_then(|r| r.rsplit('/').next()) {
        Some(name) => resolve(&defs[name], defs),
        None => schema,
    }
}

/// `text` in lower-case words, so a field's label (Keep groups) reads as its argument's name
/// (`keep_groups`) does.
fn words(text: &str) -> String {
    text.to_lowercase().replace(['_', '.'], " ")
}

/// Over the whole catalog, a form asking all of a command's arguments draws a text field only
/// for an argument that is no choice (a path, free text or an unbounded number, or one
/// number of a vector), and every field it draws is one of the command's arguments.
#[test]
fn over_the_whole_catalog_only_a_value_is_typed() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let catalog = ui.host.lock().shell.catalog.clone();
    let mut typed = 0;
    for info in &catalog.commands {
        let schema = &info.args_schema;
        let Some(properties) = schema["properties"].as_object() else {
            continue;
        };
        let names: Vec<&String> = properties.keys().collect();
        ui.ok("ui.palette", json!({ "form": info.name, "ask": names }));
        for field in with_role(&mut ui, "text field") {
            let label = field["label"].as_str().unwrap_or_default();
            // Its own name, else one number of a vector ("Center X").
            let exact = |n: &&&String| words(label) == words(n);
            let within = |n: &&&String| words(label).starts_with(&format!("{} ", words(n)));
            let name = (names
                .iter()
                .find(exact)
                .or_else(|| names.iter().find(within)))
            .unwrap_or_else(|| panic!("{}: a field {label} for no argument", info.name));
            let arg = &properties[name.as_str()];
            assert!(
                !choice(arg, &schema["$defs"]),
                "{}: {name} is a choice, typed in {label}",
                info.name
            );
            typed += 1;
        }
        ui.ok("ui.palette", json!({ "open": false }));
    }
    assert!(typed > 0, "the paths and names are still typed");
}
