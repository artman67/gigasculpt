//! One look command for every view (decision 0025, section 4): `view.look` sets the MatCap and
//! the edges of an agent's view, `main` and a Reference View, and the Look options and the
//! Look on the reference's bar run it. The Look options' Material section edits the active
//! material, one palette step a slider drag. The Look options take the compact rows (decision
//! 0060 as amended for R1): Shading's modes and Tone's transforms a row each, lit on the one in
//! use, and a row only while it applies.

use gs_platform::{Input, Modifiers};
use gs_protocol::ErrorCode;
use serde_json::{Value, json};

use crate::materials::actions;
use crate::region::middle;
use crate::rig::{Ui, center};
use crate::view_options::{lit, row};

/// The session's look, `main`'s edges, and the active reference's look and edges, as
/// `session.state` reports them.
fn looks(ui: &mut Ui) -> Value {
    let s = ui.state();
    json!([
        s["look"],
        s["view.wireframe"],
        s["reference.look"],
        s["reference.wireframe"]
    ])
}

/// Taps the widget labelled `label` in panel `panel`.
fn tap_on(ui: &mut Ui, panel: &str, label: &str) {
    let [x, y] = middle(ui.rect(&format!("{panel} > {label}")));
    ui.tap_at(x, y);
}

#[test]
fn one_look_command_sets_the_matcap_and_edges_of_every_view() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("reference.open", json!({ "scene": true }));
    ui.settle();
    // An agent's view has edges of its own and draws the session's look.
    let agent = json!({ "view": "agent:helper", "matcap": "chrome-a", "wireframe": true });
    assert_eq!(ui.ok("view.look", agent.clone()), agent);
    let want = json!(["chrome-a", false, "chrome-a", false]);
    assert_eq!(looks(&mut ui), want, "main's edges stay off");
    // Main's look is the session's, which a reference left on main draws too.
    ui.ok(
        "view.look",
        json!({ "matcap": "red-wax", "wireframe": true }),
    );
    assert_eq!(looks(&mut ui), json!(["red-wax", true, "red-wax", false]));
    // A reference's own look leaves main's; "main" gives it main's back.
    let own = json!({ "view": "ref:1", "matcap": "skin-4", "flip_wireframe": true });
    ui.ok("view.look", own);
    assert_eq!(looks(&mut ui), json!(["red-wax", true, "skin-4", true]));
    let back = ui.ok("view.look", json!({ "view": "ref:1", "matcap": "main" }));
    assert_eq!(back["matcap"], json!("red-wax"), "{back}");
    let wrong = json!({ "view": "ref:1", "matcap": "red wax", "wireframe": false });
    let (code, message) = ui.err("view.look", wrong);
    assert_eq!(code, ErrorCode::NotFound, "{message}");
    let want = json!(["red-wax", true, "red-wax", true]);
    assert_eq!(looks(&mut ui), want, "a refused look changes nothing");
    // The Look options pick main's look with it, and the Look on the reference's bar its own.
    ui.open_look();
    let gray = (ui.widgets("look").into_iter()).find(|w| w["label"] == "MatCap Gray");
    let gray = gray.unwrap_or_else(|| panic!("the Look options list MatCap Gray"));
    assert_eq!(gray["command"]["command"], json!("view.look"));
    tap_on(&mut ui, "look", "MatCap Gray");
    // A press outside the open options only shuts them (decision 0060 as amended for H5).
    ui.tap("Escape", Modifiers::default());
    tap_on(&mut ui, "reference", "Look");
    // The menu scrolls past its first few rows; White is in sight.
    tap_on(&mut ui, "reference", "MatCap White");
    let want = json!(["gray", true, "white", true]);
    assert_eq!(looks(&mut ui), want, "each picked for its own view");
}

/// The words beside the Look on the reference's bar.
fn reference_look_words(ui: &mut Ui) -> String {
    let labels = ui.labels("reference");
    let look = labels.iter().position(|l| l == "Look");
    let after = look.and_then(|i| labels.get(i + 1));
    after
        .cloned()
        .unwrap_or_else(|| panic!("no Look in {labels:?}"))
}

#[test]
fn the_look_on_the_reference_bar_says_which_look_is_on() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("view.look", json!({ "matcap": "gray" }));
    ui.ok("reference.open", json!({ "scene": true }));
    ui.settle();
    // Following the view, the reference draws with the view's look, and its Look says so.
    assert_eq!(reference_look_words(&mut ui), "MatCap Gray");
    // The layout's docked reference wraps its bar, and Look and its title wrap together.
    let (look, words) = (
        ui.rect("reference > Look"),
        ui.rect("reference > MatCap Gray"),
    );
    assert!(
        words[1] >= look[1] && words[3] <= look[3],
        "{look:?} {words:?}"
    );
    tap_on(&mut ui, "reference", "Look");
    tap_on(&mut ui, "reference", "MatCap White");
    ui.settle();
    assert_eq!(reference_look_words(&mut ui), "MatCap White");
}

/// Whether the row `label` of the reference's Look menu, open, is lit.
fn lit_row(ui: &mut Ui, label: &str) -> bool {
    let widgets = ui.widgets("reference");
    let row = (widgets.iter()).find(|w| w["label"] == label && w["role"] == "button");
    row.unwrap_or_else(|| panic!("no {label} row in {widgets:?}"))["selected"] == json!(true)
}

/// The Look menu on the reference's bar lights the row in use (decision 0025, section 4,
/// amended 2026-10-14, UI-last): "Same as the view" while the reference follows the view's
/// look, its own look once one is picked. `reference.matcap` says which, as `view.look` takes
/// it, and `reference.look` still names the look it draws with.
#[test]
fn the_reference_look_menu_lights_same_as_the_view_while_it_follows() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("view.look", json!({ "matcap": "gray" }));
    ui.ok("reference.open", json!({ "scene": true }));
    ui.settle();
    tap_on(&mut ui, "reference", "Look");
    assert!(lit_row(&mut ui, "Same as the view"), "it follows the view");
    assert!(!lit_row(&mut ui, "MatCap Gray"), "gray is the view's");
    let s = ui.state();
    assert_eq!(
        [&s["reference.matcap"], &s["reference.look"]],
        [&json!("main"), &json!("gray")]
    );
    tap_on(&mut ui, "reference", "MatCap White");
    ui.settle();
    tap_on(&mut ui, "reference", "Look");
    assert!(lit_row(&mut ui, "MatCap White"), "its own");
    assert!(!lit_row(&mut ui, "Same as the view"));
    let s = ui.state();
    assert_eq!(
        [&s["reference.matcap"], &s["reference.look"]],
        [&json!("white"), &json!("white")]
    );
}

/// The active material's values, as `material.list` reports them.
fn active_values(ui: &mut Ui) -> Value {
    let list = ui.ok("material.list", json!({}));
    let looks = list["looks"].as_array().cloned().unwrap_or_default();
    let active = looks.into_iter().find(|m| m["name"] == list["selected"]);
    active.map(|m| m["values"].clone()).unwrap_or_default()
}

/// The main view's picture of the scene, from the front.
fn picture(ui: &mut Ui) -> Value {
    let args = json!({ "view": "front", "size": 96, "antialias": false });
    ui.ok("view.snapshot", args)["png_base64"].clone()
}

/// The Look options' Material section (decision 0043, section 8, step (b)): a drag of one of
/// the active material's sliders runs `material.set` once, when the pen lets go, as one palette
/// step, and the section's Undo puts the picture back byte for byte.
#[test]
fn a_material_slider_drag_is_one_palette_step_that_undo_takes_back() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    let (before, values) = (picture(&mut ui), active_values(&mut ui));
    ui.open_look();
    ui.scroll_to("look", "Specular");
    let r = ui.rect("look > Specular");
    let [x, y] = [r[0] + 20.0, middle(r)[1]];
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    ui.touch(x - 400.0, y, true);
    let held = ui.logged("material.set");
    assert!(
        held.is_empty(),
        "nothing runs while the pen drags: {held:?}"
    );
    ui.touch(x - 400.0, y, false);
    assert_eq!(
        ui.logged("material.set").len(),
        1,
        "one call, at the release"
    );
    let mut want = values.clone();
    want["specular_weight"] = json!(0.0);
    assert_eq!(
        active_values(&mut ui),
        want,
        "the specular weight alone, to the rail's end"
    );
    assert_ne!(picture(&mut ui), before, "the edited look draws");

    actions(&mut ui);
    ui.press("look > Undo");
    let more = ui.dry_run("material.undo", json!({}));
    assert_eq!(
        more,
        Some(ErrorCode::Precondition),
        "the drag was one palette step"
    );
    assert_eq!(active_values(&mut ui), values);
    assert!(
        picture(&mut ui) == before,
        "the picture is back byte for byte"
    );
    // The window's dry run greys Reset on a copy, which has no shipped settings.
    ui.ok("material.copy", json!({ "name": "mine" }));
    let reset = json!({ "material": "mine", "shipped": true });
    let greyed = ui.dry_run("material.set", reset);
    assert_eq!(greyed, Some(ErrorCode::Precondition), "Reset on a copy");
}

/// Scrolls the Look options to their foot, the Material section's end: the wheel over a label,
/// and the time egui's smooth scroll takes.
fn to_foot(ui: &mut Ui) {
    let heading = (ui.widgets("look").into_iter()).find(|w| w["role"] == "label");
    let [x, y] = center(&heading.unwrap_or_else(|| panic!("the Look options show"))["rect"]);
    ui.pen(x, y);
    let wheel = Input::Wheel {
        dx: 0.0,
        dy: -10_000.0,
    };
    ui.input(&[wheel], Modifiers::default());
    ui.wait(1.0);
}

/// The Material section's sliders at the Look options' foot, last first.
fn material_sliders(ui: &mut Ui) -> Vec<String> {
    to_foot(ui);
    let widgets = ui.widgets("look");
    (widgets.iter().rev())
        .take_while(|w| w["role"] == "slider")
        .filter_map(|w| Some(w["label"].as_str()?.to_string()))
        .collect()
}

/// The cavity sliders (decision 0043, step (c)) show only for a material they act on, a
/// standard one: a MatCap look shows the nine sliders without them, and Flat Color none. A drag
/// of each is one `material.set` at the release and one palette step.
#[test]
fn the_cavity_sliders_show_for_a_standard_material_and_each_drag_is_one_step() {
    const CAVITY: [&str; 3] = ["Cavity intensity", "Cavity radius", "Cavity colorize"];
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.open_look();
    let matcap = material_sliders(&mut ui);
    assert!(
        matcap.first().is_some_and(|s| s == "Scatter radius"),
        "a MatCap look ends on its nine: {matcap:?}"
    );
    ui.ok("material.select", json!({ "name": "flat-color" }));
    ui.settle();
    let flat = material_sliders(&mut ui);
    assert!(flat.is_empty(), "Flat Color has none: {flat:?}");
    ui.ok("material.select", json!({ "name": "basic-material" }));
    ui.settle();
    let standard = material_sliders(&mut ui);
    assert_eq!(
        standard[..4],
        [
            "Cavity colorize",
            "Cavity radius",
            "Cavity intensity",
            "Scatter radius"
        ],
        "a standard material: the nine, then the cavity sliders"
    );
    let values = active_values(&mut ui);
    let mut want = values.clone();
    // Intensity and Radius to the rail's left end, Colorize (0 by default) to its right.
    for (title, key, end, to) in [
        (CAVITY[0], "cavity_intensity", -400.0, json!(0.0)),
        (CAVITY[1], "cavity_radius", -400.0, json!(0.25)),
        (CAVITY[2], "cavity_colorize", 400.0, json!(1.0)),
    ] {
        let calls = ui.logged("material.set").len();
        // On the rail, which starts the row; the title after it takes a typed value.
        let r = ui.rect(&format!("look > {title}"));
        let [x, y] = [r[0] + 20.0, middle(r)[1]];
        ui.touch(x, y, false);
        ui.touch(x, y, true);
        ui.touch(x + end, y, true);
        ui.touch(x + end, y, false);
        assert_eq!(
            ui.logged("material.set").len(),
            calls + 1,
            "{title}: one call"
        );
        want[key] = to;
        assert_eq!(active_values(&mut ui), want, "{title}: its key alone");
    }
    for _ in CAVITY {
        ui.ok("material.undo", json!({}));
    }
    assert_eq!(active_values(&mut ui), values, "three drags, three steps");
}

#[test]
fn the_shading_modes_and_the_view_transforms_are_a_row_each_lit_on_the_one_in_use() {
    const MODES: [&str; 5] = ["MatCap", "Lit", "Clay", "Zebra", "Normals"];
    const TONES: [&str; 3] = ["PBR Neutral", "AgX", "Standard"];
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.open_look();
    assert_eq!(row(&mut ui, "look", "MatCap"), MODES);
    assert_eq!(lit(&mut ui, "look", &MODES), ["MatCap"]);
    let widgets = ui.widgets("look");
    for mode in MODES {
        let w = widgets.iter().find(|w| w["label"] == mode);
        let shading = json!({ "shading": mode.to_lowercase() });
        let want = json!({ "command": "view.display", "args": shading });
        assert_eq!(
            w.map(|w| &w["command"]),
            Some(&want),
            "{mode} runs as before"
        );
    }
    ui.press("look > Clay");
    assert_eq!(ui.state()["view.shading"], json!("clay"));
    assert_eq!(
        lit(&mut ui, "look", &MODES),
        ["Clay"],
        "lit on the mode in use"
    );
    ui.scroll_to("look", "AgX");
    assert_eq!(row(&mut ui, "look", "AgX"), TONES);
    assert_eq!(
        lit(&mut ui, "look", &TONES).len(),
        1,
        "{TONES:?}: one in use"
    );
}

#[test]
fn a_look_row_shows_only_while_it_applies() {
    const LIT: [&str; 3] = ["Subsurface strength", "Studio", "Key light"];
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({}));
    ui.open_look();
    assert_eq!(ui.state()["view.shading"], json!("matcap"));
    assert!(ui.shows("look", "Flip MatCap"), "MatCap mode's flip");
    for waits in LIT {
        assert!(!ui.shows("look", waits), "{waits} waits for the light");
    }
    ui.scroll_to("look", "Lit");
    ui.press("look > Lit");
    assert!(!ui.shows("look", "Flip MatCap"), "no MatCap, no flip");
    for shows in LIT {
        assert!(ui.shows("look", shows), "{shows} in Lit mode");
    }

    // The mask's switch keeps its key; its opacity shows while the mask does.
    ui.scroll_to("look", "Mask");
    let mask = ui
        .widgets("look")
        .into_iter()
        .find(|w| w["label"] == "Mask");
    assert_eq!(mask.map(|w| w["key"].clone()), Some(json!("Ctrl+H")));
    ui.press("look > Mask");
    assert_eq!(ui.state()["view.mask"], json!(false));
    assert!(!ui.shows("look", "Mask opacity"), "no mask, no opacity");
    ui.press("look > Mask");
    assert!(ui.shows("look", "Mask opacity"), "back with the mask");

    // Occlusion's radius waits for some strength, the gradient's center for some gradient.
    ui.ok("view.display", json!({ "occlusion_strength": 0.0 }));
    ui.settle();
    assert!(!ui.shows("look", "Occlusion radius") && !ui.shows("look", "Auto"));
    ui.ok("view.display", json!({ "occlusion_strength": 0.5 }));
    ui.settle();
    assert!(ui.shows("look", "Occlusion radius") && ui.shows("look", "Auto"));
    assert!(
        !ui.shows("look", "Gradient center"),
        "no gradient, no center"
    );
    ui.ok("view.background", json!({ "gradient": 0.5 }));
    ui.settle();
    assert!(ui.shows("look", "Gradient center"), "a gradient's center");
}
