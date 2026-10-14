//! Brush tray > Paint and the Parts list's paint icon (decision 0039, sections 10 and 11, as
//! amended 2026-10-12 in step P8a; decision 0060 section 6, item 7): the section shows the
//! palette's Main and Secondary with Swap and Paint's ⋯ on their row, the color's Hex, Red, Green
//! and Blue only while it is open; a typed hex sets the edited one with one `color.set` and no
//! undo step, Swap and its key trade them, Color Strength reads as a percentage, Gradient and
//! Alternate light from the palette and flip it, the ⋯'s Fill part and Fill visible send the
//! brush's Color Strength as one undo step, the brush's color switch and Color Strength show
//! while they apply, folded the title says both colors, and a part row's paint icon hides and
//! shows that part's paint in the 3D view, Shift on it setting every part in one step.

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::pen::widget;
use crate::region::middle;
use crate::rig::{Ui, center, held};

pub(crate) const RED: &str = "#ff0000";
pub(crate) const BLUE: &str = "#0000ff";

/// What `ui.describe` says of the brush tray's widget `label`, scrolled into sight.
pub(crate) fn seen(ui: &mut Ui, label: &str) -> Value {
    ui.scroll_to("brush", label);
    widget(ui, "brush", label)
}

/// Whether the brush tray shows a widget `label`.
pub(crate) fn shows(ui: &mut Ui, label: &str) -> bool {
    ui.scroll_to("brush", label);
    ui.labels("brush").contains(&label.to_string())
}

/// Whether the brush tray's widget `label` is lit.
fn lit(ui: &mut Ui, label: &str) -> bool {
    seen(ui, label)["selected"] == true
}

/// The hover text of the brush tray's widget `label`.
pub(crate) fn hint(ui: &mut Ui, label: &str) -> String {
    let w = seen(ui, label);
    w["hint"].as_str().unwrap_or_default().to_string()
}

/// How many steps History can undo.
pub(crate) fn undos(ui: &mut Ui) -> Value {
    ui.state()["history.undos"].clone()
}

/// The palette's `color.main` and `color.secondary`.
pub(crate) fn colors(ui: &mut Ui) -> (Value, Value) {
    let state = ui.state();
    (
        state["color.main"].clone(),
        state["color.secondary"].clone(),
    )
}

/// Opens the edited color if it is shut, taps its hex field, types `text` and presses `key`
/// (`Return` or `Escape`).
pub(crate) fn type_hex(ui: &mut Ui, text: &str, key: &str) {
    if !shows(ui, "Hex") {
        let edited = if lit(ui, "Main") { "Main" } else { "Secondary" };
        ui.press(&format!("brush > {edited}"));
    }
    let [x, y] = middle(ui.rect("brush > Hex"));
    ui.tap_at(x, y);
    let none = Modifiers::default();
    ui.input(&[Input::Text(text.into())], none);
    ui.tap(key, none);
}

#[test]
fn the_section_shows_both_colors_and_a_typed_hex_sets_the_edited_one_with_no_undo_step() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("color.set", json!({ "main": RED, "secondary": BLUE }));
    assert!(hint(&mut ui, "Main").contains(RED), "Main says its color");
    assert!(hint(&mut ui, "Secondary").contains(BLUE));
    assert!(
        lit(&mut ui, "Main") && !lit(&mut ui, "Secondary"),
        "Main is edited at first"
    );
    let (sets, undone) = (ui.logged("color.set").len(), undos(&mut ui));
    type_hex(&mut ui, "3366CC", "Return");
    assert_eq!(ui.logged("color.set").len(), sets + 1, "one color.set");
    assert_eq!(colors(&mut ui), (json!("#3366cc"), json!(BLUE)));
    assert_eq!(undos(&mut ui), undone, "a color is no undo step");
    assert!(
        hint(&mut ui, "Main").contains("#3366cc"),
        "the swatch shows it"
    );
    type_hex(&mut ui, "#00ff00", "Escape");
    assert_eq!(
        ui.logged("color.set").len(),
        sets + 1,
        "Escape sends nothing"
    );
    // A tap on Secondary makes it the one edited: lit, and what the field sets.
    ui.press("brush > Secondary");
    type_hex(&mut ui, "#00ff00", "Return");
    assert_eq!(colors(&mut ui), (json!("#3366cc"), json!("#00ff00")));
    assert!(lit(&mut ui, "Secondary") && !lit(&mut ui, "Main"));
    assert_eq!(undos(&mut ui), undone);
}

#[test]
fn swap_by_button_and_by_key_trades_main_and_secondary() {
    let mut ui = Ui::new();
    ui.ok("color.set", json!({ "main": RED, "secondary": BLUE }));
    let swap = seen(&mut ui, "Swap");
    assert_eq!(swap["command"]["args"], json!({ "swap": true }), "{swap}");
    assert_eq!(swap["key"], "Shift+X", "the button shows its key");
    let sets = ui.logged("color.set").len();
    ui.press("brush > Swap");
    assert_eq!(colors(&mut ui), (json!(BLUE), json!(RED)));
    assert!(hint(&mut ui, "Main").contains(BLUE) && hint(&mut ui, "Secondary").contains(RED));
    ui.tap("X", held("Shift"));
    assert_eq!(
        colors(&mut ui),
        (json!(RED), json!(BLUE)),
        "Shift+X swaps back"
    );
    assert_eq!(ui.logged("color.set").len(), sets + 2, "one color.set each");
}

/// Paint's ⋯, told from the tray's other ⋯ by its hint.
const MORE: &str = "Fill part and Fill visible.";

/// Paint's ⋯ button, scrolled into sight.
fn menu(ui: &mut Ui) -> Value {
    ui.scroll_to("brush", "Swap");
    let mut all = ui.widgets("brush").into_iter();
    let more = all.rfind(|w| w["label"] == "···" && w["hint"] == MORE);
    more.unwrap_or_else(|| panic!("Paint's ⋯ shows"))
}

/// Opens Paint's ⋯ and returns what `ui.describe` says of its control `label`.
fn in_menu(ui: &mut Ui, label: &str) -> Value {
    let [x, y] = center(&menu(ui)["rect"]);
    ui.tap_at(x, y);
    widget(ui, "brush", label)
}

/// The vertical middle of the brush tray's widget `label`, and its top and foot.
fn ys(ui: &mut Ui, label: &str) -> [f64; 3] {
    let r = &widget(ui, "brush", label)["rect"];
    let at = |i: usize| r[i].as_f64().unwrap_or_default();
    [center(r)[1], at(1), at(3)]
}

/// Decision 0060, section 6, item 7: Main, Swap, Secondary and Paint's ⋯ on a row; the color's
/// Hex, Red, Green and Blue only while it is open, a tap on the other swatch making it the one
/// edited and a second tap or a hold opening it; then Color Strength, Gradient and Alternate on
/// a row, and the brush's Color switch; Fill part and Fill visible in the ⋯.
#[test]
fn the_swatches_swap_and_menu_share_a_row_and_a_second_tap_or_a_hold_opens_the_color() {
    let mut ui = Ui::new();
    ui.ok("brush.set", json!({ "brush": "Draw" }));
    let paints = json!({ "paint": { "channel": "color" } });
    ui.ok("brush.set", json!({ "settings": paints }));
    ui.open_only_in("brush", &["paint"]);
    let row = ys(&mut ui, "Main")[0];
    for label in ["Swap", "Secondary"] {
        assert!(
            (ys(&mut ui, label)[0] - row).abs() < 10.0,
            "{label} on Main's row"
        );
    }
    assert!(
        (center(&menu(&mut ui)["rect"])[1] - row).abs() < 10.0,
        "the ⋯ on Main's row"
    );
    let shut = ["Hex", "Red", "Green", "Blue", "Fill part", "Fill visible"];
    for label in shut {
        assert!(!shows(&mut ui, label), "{label} waits");
    }
    let strength = ys(&mut ui, "Color Strength");
    let gradient = ys(&mut ui, "Gradient");
    assert!(
        strength[1] > ys(&mut ui, "Main")[2],
        "Color Strength under the swatches"
    );
    assert!(gradient[1] > strength[2], "Gradient under Color Strength");
    assert!(
        (ys(&mut ui, "Alternate")[0] - gradient[0]).abs() < 10.0,
        "Alternate beside it"
    );
    assert!(
        ys(&mut ui, "Color")[1] > gradient[2],
        "the paint channel under them"
    );
    // A tap on Secondary edits it; a second opens it, a third shuts it; a hold on Main opens it.
    ui.press("brush > Secondary");
    assert!(
        lit(&mut ui, "Secondary") && !shows(&mut ui, "Hex"),
        "edited, still shut"
    );
    ui.press("brush > Secondary");
    assert!(
        shows(&mut ui, "Hex") && shows(&mut ui, "Red"),
        "a second tap opens it"
    );
    ui.press("brush > Secondary");
    assert!(!shows(&mut ui, "Hex"), "a third shuts it");
    ui.hold("brush > Main");
    assert!(
        lit(&mut ui, "Main") && shows(&mut ui, "Blue"),
        "a hold edits and opens it"
    );
    ui.tap("Escape", Modifiers::default());
    for label in ["Fill part", "Fill visible"] {
        assert_eq!(in_menu(&mut ui, label)["command"]["command"], "paint.fill");
        ui.tap("Escape", Modifiers::default());
    }
}

/// Color Strength reads as a percentage: 25 typed is a quarter (decision 0062).
#[test]
fn color_strength_reads_as_a_percentage() {
    let mut ui = Ui::new();
    ui.ok("brush.set", json!({ "brush": "Paint" }));
    ui.open_only_in("brush", &["paint"]);
    let r = ui.rect("brush > Color Strength");
    ui.tap_at(r[2] - 4.0, middle(r)[1]);
    let none = Modifiers::default();
    ui.input(&[Input::Text("25".into())], none);
    ui.tap("Return", none);
    let strength = &ui.state()["brush.settings"]["paint"]["color_strength"];
    assert_eq!(
        strength.as_f64().map(|v| (v * 1e4).round()),
        Some(2500.0),
        "{strength}"
    );
}

/// Folded, Paint's title says Main's and Secondary's colors; open, it says nothing.
#[test]
fn folded_paint_s_title_says_both_colors() {
    let mut ui = Ui::new();
    ui.ok("color.set", json!({ "main": RED, "secondary": BLUE }));
    let both = format!("{RED} · {BLUE}");
    ui.open_only_in("brush", &["paint"]);
    assert!(!ui.labels("brush").contains(&both), "open, no summary");
    ui.open_only_in("brush", &[]);
    assert!(
        ui.labels("brush").contains(&both),
        "{:?}",
        ui.labels("brush")
    );
}

#[test]
fn gradient_and_alternate_light_from_the_palette_and_flip_it() {
    let mut ui = Ui::new();
    assert!(!lit(&mut ui, "Gradient"), "off by default");
    assert!(lit(&mut ui, "Alternate"), "on by default");
    for (label, value) in [
        ("Gradient", "color.gradient"),
        ("Alternate", "color.alternate"),
    ] {
        for _ in 0..2 {
            let was = ui.state()[value] == true;
            ui.press(&format!("brush > {label}"));
            assert_eq!(ui.state()[value], json!(!was), "{label} flips {value}");
            assert_eq!(lit(&mut ui, label), !was, "{label} lights from it");
        }
    }
    ui.ok("color.set", json!({ "gradient": true, "alternate": false }));
    assert!(
        lit(&mut ui, "Gradient") && !lit(&mut ui, "Alternate"),
        "an agent's change"
    );
}

#[test]
fn fill_sends_the_brush_s_color_strength_and_leaves_one_undo_step() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("primitive.cube", json!({ "center": [3, 0, 0] }));
    ui.ok("color.set", json!({ "main": RED }));
    ui.ok("brush.set", json!({ "brush": "Paint" }));
    let half = json!({ "paint": { "color_strength": 0.5 } });
    ui.ok("brush.set", json!({ "settings": half }));
    let none = Modifiers::default();
    let fill = in_menu(&mut ui, "Fill part")["command"].clone();
    assert_eq!(
        fill,
        json!({ "command": "paint.fill", "args": { "strength": 0.5 } })
    );
    ui.tap("Escape", none);
    let every = in_menu(&mut ui, "Fill visible")["command"].clone();
    let args = json!({ "every": true, "strength": 0.5 });
    assert_eq!(every, json!({ "command": "paint.fill", "args": args }));
    ui.tap("Escape", none);
    for (label, n) in [("Fill part", 1), ("Fill visible", 2)] {
        let undone = undos(&mut ui).as_u64().unwrap_or_default();
        in_menu(&mut ui, label);
        ui.press(&format!("brush > {label}"));
        assert_eq!(ui.logged("paint.fill").len(), n, "{label} fills");
        assert_eq!(undos(&mut ui), json!(undone + 1), "{label}: one undo step");
    }
    // A brush that paints nothing sends no strength: the engine's 1.
    ui.ok("brush.set", json!({ "brush": "Draw" }));
    let fill = in_menu(&mut ui, "Fill part")["command"].clone();
    assert_eq!(fill["args"], json!({}), "{fill}");
}

#[test]
fn the_brush_s_color_switch_and_color_strength_show_while_they_apply() {
    let mut ui = Ui::new();
    ui.ok("brush.set", json!({ "brush": "Draw" }));
    assert!(!lit(&mut ui, "Color"), "Draw paints nothing");
    assert!(!shows(&mut ui, "Color Strength"), "nor shows its strength");
    ui.press("brush > Color");
    assert_eq!(
        ui.state()["brush.settings"]["paint"],
        json!({ "channel": "color" })
    );
    assert!(lit(&mut ui, "Color") && shows(&mut ui, "Color Strength"));
    ui.press("brush > Color");
    assert_eq!(
        ui.state()["brush.settings"].get("paint"),
        None,
        "the preset's own"
    );
    assert!(!shows(&mut ui, "Color Strength"));
    // A drag brush refuses a `[paint]` table: the switch shows, its hint says so, and the press
    // changes nothing (decision 0062).
    ui.ok("brush.set", json!({ "brush": "Move" }));
    let said = hint(&mut ui, "Color");
    assert!(said.contains("cannot paint"), "{said}");
    ui.press("brush > Color");
    let settings = ui.state()["brush.settings"].clone();
    assert_eq!(
        settings.get("paint"),
        None,
        "Move keeps no paint: {settings}"
    );
    // The Paint family always paints: no switch, and its strength.
    ui.ok("brush.set", json!({ "brush": "Paint" }));
    assert!(!shows(&mut ui, "Color") && shows(&mut ui, "Color Strength"));
}

/// The window's color at the 3D view's middle, as its last frame drew it.
fn view_pixel(ui: &mut Ui) -> [u8; 3] {
    let at = ui.middle();
    pixel_at(ui, at)
}

/// The window's color at `at`, in pixels, as its last frame drew it once the view settled.
pub(crate) fn pixel_at(ui: &mut Ui, at: [f64; 2]) -> [u8; 3] {
    ui.settle();
    ui.wait(0.2);
    let [x, y] = at.map(|v| v as usize);
    let studio = ui.host.lock();
    let render = studio
        .render
        .as_ref()
        .unwrap_or_else(|| panic!("a renderer"));
    let target = render.target().unwrap_or_else(|| panic!("a frame"));
    let image = (render.renderer.read_frame(target)).unwrap_or_else(|e| panic!("{e}"));
    let i = 4 * (y * image.width as usize + x);
    [image.rgba[i], image.rgba[i + 1], image.rgba[i + 2]]
}

/// The paint icon on `part`'s row in the scene tray.
fn icon(ui: &mut Ui, part: &str) -> Value {
    let widgets = ui.widgets("tray");
    let name = widgets
        .iter()
        .find(|w| w["label"] == part && w["role"] != "label");
    let [_, y] = center(&name.unwrap_or_else(|| panic!("no row {part}"))["rect"]);
    let on_row = |w: &&Value| w["label"] == "Paint" && (center(&w["rect"])[1] - y).abs() < 1.0;
    let found = widgets.iter().find(on_row).cloned();
    found.unwrap_or_else(|| panic!("no paint icon on {part}'s row"))
}

/// Taps the paint icon on `part`'s row.
fn tap_icon(ui: &mut Ui, part: &str) {
    let [x, y] = center(&icon(ui, part)["rect"]);
    ui.tap_at(x, y);
}

/// Each part's `paint_visible`, in list order.
fn painted(ui: &mut Ui) -> Vec<Value> {
    let list = ui.ok("part.list", json!({}))["parts"].clone();
    let parts = list.as_array().cloned().unwrap_or_default();
    parts.iter().map(|p| p["paint_visible"].clone()).collect()
}

#[test]
fn a_row_s_paint_icon_hides_its_part_s_paint_in_the_view_and_shift_sets_every_part() {
    let mut ui = Ui::with_looks();
    ui.ok("primitive.sphere", json!({ "name": "a" }));
    ui.ok(
        "primitive.sphere",
        json!({ "name": "b", "center": [3, 0, 0] }),
    );
    ui.ok("paint.fill", json!({ "every": true, "color": RED }));
    ui.open_only(&["parts"]);
    ui.ok(
        "view.frame",
        json!({ "view": "main", "target": "part", "part": "a" }),
    );
    let red = view_pixel(&mut ui);
    assert!(
        red[0] > red[1].saturating_add(60),
        "a shows its paint: {red:?}"
    );
    let a = icon(&mut ui, "a");
    assert_eq!(a["selected"], true, "lit while the paint shows");
    let args = json!({ "part": "a", "visible": false });
    assert_eq!(a["command"]["args"], args, "{a}");
    let undone = undos(&mut ui).as_u64().unwrap_or_default();
    tap_icon(&mut ui, "a");
    assert_eq!(painted(&mut ui), [false, true]);
    assert_eq!(icon(&mut ui, "a")["selected"], false, "it goes dark");
    let plain = view_pixel(&mut ui);
    let gray = (i32::from(plain[0]) - i32::from(plain[1])).abs() < 20;
    assert!(gray, "a draws without its paint: {plain:?}");
    tap_icon(&mut ui, "a");
    assert_eq!(painted(&mut ui), [true, true]);
    assert_eq!(view_pixel(&mut ui), red, "shown again unchanged");
    assert_eq!(undos(&mut ui), json!(undone + 2), "each tap one step");
    // Shift gives every part the state the tap gives its row, in one step.
    ui.input(&[], held("Shift"));
    let every = json!({ "every": true, "visible": false });
    assert_eq!(icon(&mut ui, "b")["command"]["args"], every);
    tap_icon(&mut ui, "b");
    assert_eq!(painted(&mut ui), [false, false]);
    assert_eq!(undos(&mut ui), json!(undone + 3), "one step for every part");
    tap_icon(&mut ui, "b");
    assert_eq!(painted(&mut ui), [true, true]);
    ui.input(&[], Modifiers::default());
}
