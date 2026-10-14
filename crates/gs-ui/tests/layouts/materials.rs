//! The material palette's window (decision 0043, step (d)): the Library's Materials tab pictures
//! every material by kind (a baked ball, a flat material's disc of its color) from the palette's
//! own recipes, the user folder's included, and Show Used keeps the materials the scene uses;
//! the Material section's picture opens the tab and says what the folder refused; its slot
//! slider picks a material by place; a pen resting on a tile draws the view in that material and
//! runs nothing; Copy, Rename and Delete are controls behind the section's ⋯ (decision 0060 as
//! amended for R1), Delete greyed on a built-in; each color is a color button whose pick is one
//! `material.set` when the pen lets go, and the environment a row of the shipped ones.

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::library::lit;
use crate::region::middle;
use crate::rig::{Ui, folder};

/// The Material section's ⋯ as drawn, in three middle dots: the interface's font has no ⋯.
const MORE: &str = "···";

/// Opens the Material section's ⋯, which lists the material's actions (decision 0060 as amended
/// for R1), opening the Look options first if they are shut.
pub fn actions(ui: &mut Ui) {
    if ui.widgets("look").is_empty() {
        ui.open_look();
    }
    ui.scroll_to("look", MORE);
    ui.press(&format!("look > {MORE}"));
}

/// A standard material's look file with the base color `base` (linear), titled `title`.
fn standard(title: &str, base: [f32; 3]) -> String {
    format!("name = \"{title}\"\nkind = \"standard\"\n\n[material]\nbase_color = {base:?}\n")
}

/// A window on lavapipe with a sphere, which read a `materials` folder holding `files` as the
/// window does at start (`gs_ui::app::materials`).
fn window(name: &str, files: &[(&str, &str)]) -> Ui {
    let config = folder(name);
    let step = gs_ui::app::materials(&config);
    for (file, text) in files {
        let path = config.join("materials").join(file);
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.ok(&step.command, step.args);
    ui.settle();
    ui
}

/// The window's color at `at` (pixels), as its last frame drew it.
fn pixel(ui: &mut Ui, at: [f64; 2]) -> [u8; 3] {
    let studio = ui.host.lock();
    let render = studio
        .render
        .as_ref()
        .unwrap_or_else(|| panic!("a renderer"));
    let target = render.target().unwrap_or_else(|| panic!("a frame"));
    let image = (render.renderer.read_frame(target)).unwrap_or_else(|e| panic!("{e}"));
    let [x, y] = at.map(|v| v as usize);
    let i = 4 * (y * image.width as usize + x);
    [image.rgba[i], image.rgba[i + 1], image.rgba[i + 2]]
}

/// The middle of the Materials tile titled `title`, once the frames making pictures are drawn.
fn picture(ui: &mut Ui, title: &str) -> [u8; 3] {
    ui.settle();
    ui.wait(0.2);
    let at = middle(ui.rect(&format!("library > {title}")));
    pixel(ui, at)
}

/// The titles of the Library's tiles in sight that pick a material.
fn tiles(ui: &mut Ui) -> Vec<String> {
    let widgets = ui.widgets("library");
    let looks = widgets
        .iter()
        .filter(|w| w["command"]["command"] == "view.look");
    looks
        .filter_map(|w| Some(w["label"].as_str()?.to_string()))
        .collect()
}

/// The active material as `material.list` lists it.
fn active(ui: &mut Ui) -> Value {
    let list = ui.ok("material.list", json!({}));
    let looks = list["looks"].as_array().cloned().unwrap_or_default();
    let found = looks.into_iter().find(|m| m["name"] == list["selected"]);
    found.unwrap_or_default()
}

#[test]
fn the_materials_tab_pictures_each_material_by_kind_and_show_used_keeps_the_scenes() {
    let blue = standard("Blue Glaze", [0.03, 0.06, 0.6]);
    let wax = "name = \"My Wax\"\n\n[material]\nbase_color = [0.6, 0.12, 0.06]\n";
    let files = [
        ("blue.toml", blue.as_str()),
        ("wax.toml", wax),
        ("broken.toml", "name = 3\n"),
    ];
    let mut ui = window("materials-tab", &files);

    // The Material section says what the folder refused, and its picture opens the grid.
    ui.open_look();
    ui.scroll_to("look", "1 file not read");
    let refused = ui.widget("look", "1 file not read");
    let hint = refused["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("broken.toml"), "named on hover: {refused}");
    ui.scroll_to("look", "Materials");
    ui.press("look > Materials");
    let info = ui.ok("ui.library", json!({}));
    assert_eq!(
        (&info["open"], &info["tab"]),
        (&json!(true), &json!("matcaps"))
    );
    let labels = ui.labels("library");
    for label in ["Materials", "Show Used", "Preview", "Close", "MatCap Gray"] {
        assert!(labels.iter().any(|l| l == label), "{label} in {labels:?}");
    }
    assert!(lit(&mut ui, "library", "Preview"), "the preview starts on");
    assert!(!lit(&mut ui, "library", "Show Used"));

    // Show Used: the material every part draws until material paint, the active one.
    ui.press("library > Show Used");
    let info = ui.ok("ui.library", json!({}));
    assert_eq!(
        (&info["used"], &info["preview"]),
        (&json!(true), &json!(true))
    );
    assert!(lit(&mut ui, "library", "Show Used"));
    assert_eq!(tiles(&mut ui), ["MatCap Gray"]);
    let [r, g, b] = picture(&mut ui, "MatCap Gray");
    assert!(r.abs_diff(b) < 24 && g > 60, "a gray ball: {:?}", [r, g, b]);
    ui.ok("material.select", json!({ "name": "blue" }));
    assert_eq!(tiles(&mut ui), ["Blue Glaze"], "the user folder's");
    let [r, g, b] = picture(&mut ui, "Blue Glaze");
    assert!(
        b > r + 40 && b > g + 20,
        "baked from its file: {:?}",
        [r, g, b]
    );
    ui.ok("material.select", json!({ "name": "wax" }));
    let [r, g, b] = picture(&mut ui, "My Wax");
    assert!(r > g + 40, "a MatCap look of the folder's: {:?}", [r, g, b]);
    ui.ok("material.select", json!({ "name": "flat-color" }));
    let flat = picture(&mut ui, "Flat Color");
    assert!(flat.iter().all(|&c| c >= 250), "a white disc: {flat:?}");

    // An edit is a new picture: it is kept by what it is made from, not by name.
    let green = json!({ "material": "blue", "base_color": [0.03, 0.6, 0.06] });
    ui.ok("material.set", green);
    ui.ok("material.select", json!({ "name": "blue" }));
    let [r, g, b] = picture(&mut ui, "Blue Glaze");
    assert!(g > r + 40 && g > b + 20, "as edited: {:?}", [r, g, b]);

    // Off, by command as an agent turns it: the whole palette.
    let info = ui.ok("ui.library", json!({ "used": false }));
    assert_eq!(info["used"], json!(false));
    ui.settle();
    let all = tiles(&mut ui);
    assert!(all.len() > 5 && all[0] == "MatCap Gray", "{all:?}");
}

#[test]
fn the_slot_slider_picks_a_material_by_place_once_at_the_release() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.open_look();
    ui.scroll_to("look", "Material slot");
    let slot = ui.widget("look", "Material slot");
    let rests = json!({ "command": "material.select", "args": { "name": "@0" } });
    assert_eq!(slot["command"], rests, "at the active material's place");
    let r = ui.rect("look > Material slot");
    let [x, y] = [r[0] + 20.0, middle(r)[1]];
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x + 600.0, y, true);
    let held = ui.logged("material.select");
    assert!(
        held.is_empty(),
        "nothing runs while the pen drags: {held:?}"
    );
    ui.touch(x + 600.0, y, false);
    assert_eq!(
        ui.logged("material.select").len(),
        1,
        "one call, at the release"
    );
    let list = ui.ok("material.list", json!({}));
    assert_eq!(
        list["selected"],
        json!("soft-skin-shader"),
        "the last place"
    );

    // A flat material has no settings: no colors, environment or sliders after its note, which
    // ends the Look options.
    ui.ok("material.select", json!({ "name": "flat-color" }));
    ui.settle();
    let flat = "A flat material has no settings";
    ui.scroll_to("look", flat);
    let labels = ui.labels("look");
    assert_eq!(labels.last().map(String::as_str), Some(flat), "{labels:?}");
}

#[test]
fn the_materials_actions_are_behind_its_more_button() {
    const ACTIONS: [&str; 6] = ["Undo", "Redo", "Reset", "Copy", "Rename", "Delete"];
    let listed = |ui: &mut Ui| {
        let labels = ui.labels("look");
        let shows = |a: &&str| labels.iter().any(|l| l == a);
        ACTIONS.into_iter().filter(shows).count()
    };
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.open_look();
    ui.scroll_to("look", MORE);
    let more = ui.widget("look", MORE);
    assert!(more["command"].is_null(), "it only opens the list: {more}");
    assert_eq!(listed(&mut ui), 0, "shut, none of the actions shows");
    ui.press(&format!("look > {MORE}"));
    assert_eq!(
        listed(&mut ui),
        ACTIONS.len(),
        "open, each by its short label"
    );
    let copy = ui.widget("look", "Copy");
    assert_eq!(copy["command"]["command"], json!("material.copy"), "{copy}");
    ui.press("look > Copy");
    let list = ui.ok("material.list", json!({}));
    let looks = list["looks"].as_array().cloned().unwrap_or_default();
    assert!(looks.iter().any(|m| m["name"] == "gray-copy"), "copied");
    assert_eq!(listed(&mut ui), 0, "a pick shuts the list");
    assert!(
        !ui.widgets("look").is_empty(),
        "and leaves the Look options open"
    );
}

#[test]
fn copy_rename_and_delete_are_controls_and_delete_is_greyed_on_a_built_in() {
    let mut ui = Ui::with_looks();
    actions(&mut ui);
    // A call, so the engine says which of the controls drawn cannot run.
    ui.state();
    ui.settle();
    let delete = ui.widget("look", "Delete");
    let why = delete["reason"].as_str().unwrap_or_default();
    assert!(
        delete["enabled"] == false && why.contains("built in"),
        "{delete}"
    );

    ui.press("look > Copy");
    let names = ui.ok("material.list", json!({}))["looks"].clone();
    assert!(
        (names.as_array().into_iter().flatten()).any(|m| m["name"] == "gray-copy"),
        "{names}"
    );
    ui.ok("material.select", json!({ "name": "gray-copy" }));
    ui.settle();
    // Rename asks only the title, what the window shows, in the command palette's form; the
    // name commands take stays (review 0009, R9).
    actions(&mut ui);
    ui.press("look > Rename");
    let widgets = ui.widgets("palette");
    let fields: Vec<&Value> = (widgets.iter())
        .filter(|w| w["role"] == "text field")
        .map(|w| &w["label"])
        .collect();
    assert_eq!(fields, [&json!("Title")]);
    if let Some(form) = ui.host.lock().shell.palette.form.as_mut() {
        for (_, text) in &mut form.fields {
            "Ours".clone_into(text);
        }
    }
    ui.input(&[], Modifiers::default());
    ui.press("palette > Rename material");
    let renamed = active(&mut ui);
    assert_eq!(
        (&renamed["name"], &renamed["title"]),
        (&json!("gray-copy"), &json!("Ours"))
    );

    // Delete runs on a user material: the active one goes, and the first becomes active.
    actions(&mut ui);
    ui.state();
    ui.settle();
    assert_eq!(ui.widget("look", "Delete")["enabled"], json!(true));
    ui.press("look > Delete");
    assert_eq!(active(&mut ui)["name"], json!("gray"));
    let list = ui.ok("material.list", json!({}));
    let names = list["looks"].as_array().cloned().unwrap_or_default();
    assert!(!names.iter().any(|m| m["name"] == "gray-copy"), "deleted");
}

#[test]
fn a_color_pick_and_an_environment_are_one_material_set_each() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    let before = active(&mut ui)["values"].clone();
    ui.open_look();
    ui.scroll_to("look", "Base color");
    let button = ui.widget("look", "Base color");
    assert_eq!(
        button["command"]["command"],
        json!("material.set"),
        "{button}"
    );
    ui.press("look > Base color");
    let picker = ui.rect("look > Base color picker");
    let [x, y] = middle(picker);
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x - 60.0, y - 40.0, true);
    ui.touch(x - 120.0, y - 80.0, true);
    let held = ui.logged("material.set");
    assert!(
        held.is_empty(),
        "nothing runs while the pen drags: {held:?}"
    );
    ui.touch(x - 120.0, y - 80.0, false);
    assert_eq!(
        ui.logged("material.set").len(),
        1,
        "one call, at the release"
    );
    let after = active(&mut ui)["values"].clone();
    assert_ne!(after["base_color"], before["base_color"], "picked");
    let mut want = before.clone();
    want["base_color"] = after["base_color"].clone();
    assert_eq!(after, want, "the base color alone");
    // A second tap on the button shuts the picker; Undo is behind the section's ⋯.
    ui.press("look > Base color");
    let open = ui
        .widgets("look")
        .iter()
        .any(|w| w["label"] == "Base color picker");
    assert!(!open, "the picker shut");
    assert_eq!(
        ui.logged("material.set").len(),
        1,
        "shutting it runs nothing"
    );
    actions(&mut ui);
    ui.press("look > Undo");
    assert_eq!(active(&mut ui)["values"], before, "one palette step");

    // The environment: a row of the shipped ones, the material's own lit.
    if ui.widgets("look").is_empty() {
        ui.open_look();
    }
    ui.scroll_to("look", "Lit by Photo Studio 01");
    assert!(lit(&mut ui, "look", "Lit by Studio"), "gray's own");
    ui.press("look > Lit by Photo Studio 01");
    assert_eq!(active(&mut ui)["environment"], json!("photo-studio-01"));
    assert_eq!(ui.logged("material.set").len(), 2);
    assert!(lit(&mut ui, "look", "Lit by Photo Studio 01"));
}

#[test]
fn a_pen_resting_on_a_material_tile_previews_it_in_the_view_and_runs_nothing() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("ui.library", json!({ "tab": "matcaps" }));
    ui.settle();
    let view = ui.middle();
    let gray = pixel(&mut ui, view);
    let [x, y] = middle(ui.rect("library > MatCap Red Wax"));
    ui.pen(x, y);
    ui.wait(0.2);
    assert_eq!(
        pixel(&mut ui, view),
        gray,
        "not yet: the pen has not rested"
    );
    ui.wait(0.3);
    let [r, g, b] = pixel(&mut ui, view);
    assert!(
        r > g + 40,
        "the view draws Red Wax: {:?} from {gray:?}",
        [r, g, b]
    );
    assert_eq!(ui.state()["look"], json!("gray"), "nothing chosen");
    for prefix in ["material", "view.look"] {
        assert!(ui.logged(prefix).is_empty(), "{prefix}: nothing logged");
    }
    // Off the tile, the active material again.
    let [cx, cy] = middle(ui.rect("library > Close"));
    ui.pen(cx, cy);
    assert_eq!(pixel(&mut ui, view), gray, "the active material again");

    // Preview off (its toggle beside Show Used, or `ui.library {preview: false}`).
    ui.press("library > Preview");
    assert_eq!(ui.ok("ui.library", json!({}))["preview"], json!(false));
    ui.pen(x, y);
    ui.wait(0.6);
    assert_eq!(pixel(&mut ui, view), gray, "off, a rest previews nothing");
}
