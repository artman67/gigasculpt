//! The header bar along the top of the 3D view (decision 0060, sections 2, 3, 12 and 13): its
//! menu titles open their menus on a tap, a pick runs and shuts the menu, each item shows its
//! key at its right, a disabled item says why, and a press outside a menu shuts it and does
//! nothing else. The bar wraps when narrow and never scrolls, its ends swap for the left hand,
//! a dropdown panel stays open after a change, and `ui.menu` opens a menu as a key or an agent
//! would. The view strip's buttons sit at its pen end, keeping their commands and keys, and
//! their options open under them; the Reference View's bar is in the header's style (decision
//! 0060 as amended for H5).

use gs_platform::Modifiers;
use gs_protocol::ErrorCode;
use gs_ui::RefPlace;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, listed};
use crate::shelf::rect;

/// The view buttons at the header's pen end, in their order: the strip's, then Look, then
/// Symmetry (its line's label, or the one button it folds into in a narrow view).
pub(crate) const VIEW_BUTTONS: [&str; 8] = [
    "Frame",
    "Perspective",
    "Wire",
    "Floor",
    "Lock",
    "Isolate",
    "Look",
    "Symmetry",
];

/// The workbench shown with a header of the menus `menus` and the pen end `show`, put in as a
/// user's layout file would put it; the shipped options buttons `show` names stay.
pub(crate) fn with_header(ui: &mut Ui, menus: &[&str], show: &[&str]) {
    let mut studio = ui.host.lock();
    let directions = &mut studio.shell.directions;
    let shipped = directions.get("workbench");
    let shipped = shipped.unwrap_or_else(|| panic!("the workbench"));
    let text = gs_ui::to_toml(shipped).unwrap_or_else(|e| panic!("{e}"));
    let mut file: toml::Table = toml::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
    let names =
        |n: &[&str]| toml::Value::from(n.iter().map(ToString::to_string).collect::<Vec<_>>());
    let mut header = (file.get("header").and_then(toml::Value::as_table))
        .cloned()
        .unwrap_or_default();
    header.insert("menus".into(), names(menus));
    header.insert("show".into(), names(show));
    if let Some(toml::Value::Array(options)) = header.get_mut("options") {
        options.retain(|o| show.contains(&o["id"].as_str().unwrap_or_default()));
    }
    file.insert("header".into(), toml::Value::Table(header));
    let text = toml::to_string(&file).unwrap_or_else(|e| panic!("{e}"));
    let added = directions.add("workbench.toml", &text);
    added.unwrap_or_else(|e| panic!("a header in the workbench: {e}"));
    drop(studio);
    ui.settle();
}

/// Panel `id` as `ui.describe` reports it, if it shows.
pub(crate) fn panel(ui: &mut Ui, id: &str) -> Option<Value> {
    let d = ui.describe();
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    panels.into_iter().find(|p| p["id"] == id)
}

/// The title of the menu open now, if one is.
pub(crate) fn shown_menu(ui: &mut Ui) -> Option<String> {
    let menu = panel(ui, "menu")?;
    Some(menu["title"].as_str().unwrap_or_default().to_string())
}

/// Opens the header's menu titled `title` with a tap on it, unless it is open already.
pub(crate) fn open(ui: &mut Ui, title: &str) {
    if shown_menu(ui).as_deref() != Some(title) {
        ui.press(&format!("header > {title}"));
    }
    assert_eq!(shown_menu(ui).as_deref(), Some(title), "{title} opened");
}

/// The open menu's rows (every widget but its headings and keys), top to bottom.
pub(crate) fn items(ui: &mut Ui) -> Vec<Value> {
    let widgets = ui.widgets("menu");
    widgets
        .into_iter()
        .filter(|w| w["role"] != "label")
        .collect()
}

/// The open menu's first row labelled `label`.
pub(crate) fn item(ui: &mut Ui, label: &str) -> Value {
    let rows = items(ui);
    let found = rows.iter().find(|w| w["label"] == label).cloned();
    found.unwrap_or_else(|| panic!("the menu lists {label}: {rows:?}"))
}

/// A widget's label and where it shows.
type Shown = Vec<(String, [f64; 4])>;

/// The header's two ends, with where each widget shows: its menu titles (the widgets that run
/// `ui.menu`), and the rest, at its pen end.
pub(crate) fn titles_and_ends(ui: &mut Ui) -> (Shown, Shown) {
    let widgets = ui.widgets("header");
    let (titles, ends): (Vec<&Value>, Vec<&Value>) =
        (widgets.iter()).partition(|w| w["command"]["command"] == "ui.menu");
    let shown = |of: Vec<&Value>| -> Shown {
        let label = |w: &Value| w["label"].as_str().unwrap_or_default().to_string();
        (of.into_iter())
            .map(|w| (label(w), rect(&w["rect"])))
            .collect()
    };
    (shown(titles), shown(ends))
}

/// The labels of `shown`, left to right.
pub(crate) fn left_to_right(shown: &[(String, [f64; 4])]) -> Vec<String> {
    let mut sorted = shown.to_vec();
    sorted.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
    sorted.into_iter().map(|(label, _)| label).collect()
}

/// The labels of `shown`.
fn labels(shown: &[(String, [f64; 4])]) -> Vec<String> {
    shown.iter().map(|(label, _)| label.clone()).collect()
}

/// A drag of the pen from (`x`, `y`) 60 pixels to the right, as a stroke on the model is.
pub(crate) fn drag(ui: &mut Ui, x: f64, y: f64) {
    ui.touch(x, y, false);
    for dx in [0.0, 20.0, 40.0, 60.0] {
        ui.touch(x + dx, y, true);
    }
    ui.touch(x + 60.0, y, false);
}

/// Section 2's Opening: a tap on File opens its menu under it, File lit while it is open; a
/// pick runs (New… opens the palette's form for `file.new`, as the tray's New did) and shuts
/// the menu; while one menu is open the pointer over another title opens that one.
#[test]
fn a_menu_opens_on_a_tap_runs_an_item_and_closes() {
    let mut ui = Ui::new();
    assert_eq!(shown_menu(&mut ui), None, "no menu shows at first");
    ui.press("header > File");
    assert_eq!(
        shown_menu(&mut ui).as_deref(),
        Some("File"),
        "a tap opens it"
    );
    let file = ui
        .widgets("header")
        .into_iter()
        .find(|w| w["label"] == "File");
    let file = file.unwrap_or_else(|| panic!("the header shows File"));
    assert_eq!(file["selected"], true, "File is lit while open: {file}");
    let (at, menu) = (
        rect(&file["rect"]),
        rect(&panel(&mut ui, "menu").unwrap_or_default()["rect"]),
    );
    assert!(
        menu[1] >= at[3] - 1.0 && (menu[0] - at[0]).abs() <= 4.0,
        "the menu {menu:?} opens under its title {at:?}"
    );
    ui.press("menu > New…");
    assert_eq!(shown_menu(&mut ui), None, "a pick shuts the menu");
    let form = ui
        .host
        .lock()
        .shell
        .palette
        .form
        .as_ref()
        .map(|f| f.step.command.clone());
    assert_eq!(
        form.as_deref(),
        Some("file.new"),
        "New… opens the form first"
    );
    ui.ok("ui.palette", json!({ "open": false }));
    with_header(&mut ui, &["file", "tone"], &[]);
    ui.press("header > File");
    let [x, y] = middle(ui.rect("header > Tone"));
    ui.pen(x, y);
    assert_eq!(
        shown_menu(&mut ui).as_deref(),
        Some("Tone"),
        "the pointer over Tone opens it while File is open"
    );
}

/// Section 2's A menu: each item's key, the keymap's, shows at the right of its row, and is the
/// user's own once they bind another; an item with no key shows none.
#[test]
fn each_items_key_shows_at_its_right() {
    let mut ui = Ui::new();
    open(&mut ui, "File");
    let keyed = [
        ("New…", "Ctrl+N"),
        ("Open…", "Ctrl+O"),
        ("Save", "Ctrl+S"),
        ("Save next", "Ctrl+Alt+S"),
        ("Quick save", "9"),
        ("Revert…", "Ctrl+Alt+R"),
        ("Save…", "Ctrl+Shift+T"),
    ];
    let widgets = ui.widgets("menu");
    for (label, key) in keyed {
        let row = item(&mut ui, label);
        assert_eq!(row["key"], key, "{row}");
        let at = rect(&row["rect"]);
        let shown = widgets
            .iter()
            .filter(|w| w["role"] == "label" && w["label"] == key);
        let right = shown.map(|w| rect(&w["rect"])).any(|k| {
            let within = k[1] >= at[1] - 0.5 && k[3] <= at[3] + 0.5 && k[2] <= at[2] + 0.5;
            within && k[0] > f64::midpoint(at[0], at[2])
        });
        assert!(
            right,
            "{label}'s {key} shows at its row's right, {at:?}: {widgets:?}"
        );
    }
    for label in [
        "Open recent",
        "Save as…",
        "Import…",
        "Export…",
        "Send",
        "Read",
    ] {
        assert_eq!(
            item(&mut ui, label)["key"],
            Value::Null,
            "{label} has no key"
        );
    }
    let nexts: Vec<Value> = (items(&mut ui).into_iter())
        .filter(|w| w["label"] == "Save next")
        .map(|w| w["key"].clone())
        .collect();
    assert_eq!(
        nexts,
        [json!("Ctrl+Alt+S"), Value::Null],
        "the Part's Save next has none"
    );
    let bind = json!({
        "bind": "Ctrl+Shift+I",
        "command": "ui.palette",
        "args": { "form": "file.import" },
        "replace": true,
    });
    ui.ok("ui.keymap", bind);
    ui.settle();
    assert_eq!(
        item(&mut ui, "Import…")["key"],
        "Ctrl+Shift+I",
        "the user's own key"
    );
}

/// Section 2's A menu and rule 7: Send in an empty scene is gray (disabled) and its hint is
/// why; with a part it can run, and its hint says what it does.
#[test]
fn a_disabled_item_is_gray_and_its_hint_says_why() {
    let mut ui = Ui::new();
    open(&mut ui, "File");
    let send = item(&mut ui, "Send");
    let why = json!("the scene has no parts to send");
    assert_eq!(send["command"]["command"], "exchange.write", "{send}");
    assert_eq!(
        (&send["enabled"], &send["reason"], &send["hint"]),
        (&json!(false), &why, &why),
        "{send}"
    );
    ui.ok("primitive.cube", json!({ "subdivisions": 2 }));
    ui.settle();
    let send = item(&mut ui, "Send");
    assert_eq!(send["enabled"], true, "{send}");
    let hint = send["hint"].as_str().unwrap_or_default();
    assert!(hint.starts_with("Writes every part"), "{send}");
}

/// Section 2's Opening and section 17's item 10: a press outside an open menu shuts it and
/// does nothing else: on the model it starts no stroke (the same drag with no menu open does),
/// on a tray's button it runs nothing. Escape shuts it too.
#[test]
fn a_press_outside_an_open_menu_runs_nothing_and_starts_no_stroke() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let undos = |ui: &mut Ui| ui.state()["history.undos"].clone();
    let before = undos(&mut ui);
    open(&mut ui, "File");
    let [x, y] = ui.middle();
    drag(&mut ui, x - 30.0, y);
    assert_eq!(shown_menu(&mut ui), None, "the press shut the menu");
    assert_eq!(undos(&mut ui), before, "and started no stroke");
    drag(&mut ui, x - 30.0, y);
    assert_ne!(
        undos(&mut ui),
        before,
        "with no menu open the same drag strokes"
    );
    let before = undos(&mut ui);
    ui.open_only(&["history"]);
    open(&mut ui, "File");
    let tray = ui.widgets("tray");
    let undo = tray
        .iter()
        .find(|w| w["command"]["command"] == "history.undo");
    let [ux, uy] = middle(rect(
        &undo.unwrap_or_else(|| panic!("Undo: {tray:?}"))["rect"],
    ));
    ui.tap_at(ux, uy);
    ui.settle();
    assert_eq!(shown_menu(&mut ui), None, "the press shut the menu");
    assert_eq!(undos(&mut ui), before, "and Undo did not run");
    open(&mut ui, "File");
    ui.tap("Escape", Modifiers::default());
    assert_eq!(shown_menu(&mut ui), None, "Escape shuts it");
    assert_eq!(undos(&mut ui), before, "and runs nothing");
}

/// Section 2's Too narrow: at full width the bar is one row of 40 points; narrow, the pen-end
/// group moves under the menus, the bar grows and holds every widget, nothing scrolls, the
/// menus keep their order, and the view's picture still starts under the bar, as wide as it.
#[test]
fn the_bar_wraps_to_a_second_row_when_narrow_and_never_scrolls() {
    let mut ui = Ui::new();
    with_header(&mut ui, &["file", "tone", "lights"], &["frame"]);
    let fits = |ui: &mut Ui| {
        let bar = rect(&panel(ui, "header").unwrap_or_default()["rect"]);
        let view = rect(&panel(ui, "viewport").unwrap_or_default()["rect"]);
        assert!(
            view[1] >= bar[3] - 0.5,
            "the view {view:?} starts under the bar {bar:?}"
        );
        let sides = (view[0] - bar[0]).abs().max((view[2] - bar[2]).abs());
        assert!(
            sides <= 0.5,
            "the bar {bar:?} is as wide as the view {view:?}"
        );
        let widgets = ui.widgets("header");
        assert!(
            !widgets.iter().any(|w| w["role"] == "scroll bar"),
            "it never scrolls"
        );
        for w in &widgets {
            let at = rect(&w["rect"]);
            let inside = at[0] >= bar[0] - 0.5 && at[2] <= bar[2] + 0.5;
            assert!(
                inside && at[1] >= bar[1] - 0.5 && at[3] <= bar[3] + 0.5,
                "{w} in {bar:?}"
            );
        }
        bar
    };
    let bar = fits(&mut ui);
    assert!(bar[3] - bar[1] <= 40.5, "one row at full width: {bar:?}");
    let (titles, ends) = titles_and_ends(&mut ui);
    let tops: Vec<f64> = (titles.into_iter().chain(ends))
        .map(|(_, r)| r[1])
        .collect();
    assert!(
        tops.iter().all(|t| (t - tops[0]).abs() <= 1.0),
        "one row: {tops:?}"
    );
    // Frame's options read Mask, Unmasked and Last edit since R1, short enough that the bar
    // holds one row at 1000 points wide.
    ui.host.lock().resize([800, 1100], 1.0);
    ui.settle();
    ui.settle();
    let bar = fits(&mut ui);
    assert!(bar[3] - bar[1] >= 79.5, "a second row when narrow: {bar:?}");
    let (titles, ends) = titles_and_ends(&mut ui);
    assert_eq!(
        labels(&titles),
        ["File", "Tone", "Lights"],
        "the menus keep their order"
    );
    let menus_bottom = titles.iter().map(|(_, r)| r[3]).fold(f64::MIN, f64::max);
    assert!(!ends.is_empty(), "the pen end shows");
    for (label, at) in &ends {
        assert!(
            at[1] >= menus_bottom - 0.5,
            "{label} {at:?} moved under the menus"
        );
    }
}

/// Section 12: for the right hand the menus sit at the left and the pen end at the right; for
/// the left hand the two swap ends, and each keeps its own order, so File and Frame stay first.
/// Look's options open under it, toward the view and inside it (amended for H5, item 6).
#[test]
fn the_left_hand_swaps_the_bars_ends_each_keeping_its_order() {
    let mut ui = Ui::new();
    with_header(&mut ui, &["file", "tone"], &["view", "look"]);
    let check = |ui: &mut Ui, menus_first: bool| {
        let (titles, ends) = titles_and_ends(ui);
        assert_eq!(left_to_right(&titles), ["File", "Tone"], "File stays first");
        assert_eq!(
            left_to_right(&ends),
            VIEW_BUTTONS[..7],
            "the pen end keeps its order"
        );
        let menus_right = titles.iter().map(|(_, r)| r[2]).fold(f64::MIN, f64::max);
        let menus_left = titles.iter().map(|(_, r)| r[0]).fold(f64::MAX, f64::min);
        for (label, at) in &ends {
            let apart = if menus_first {
                at[0] >= menus_right
            } else {
                at[2] <= menus_left
            };
            assert!(apart, "{label} {at:?} against the menus {titles:?}");
        }
    };
    check(&mut ui, true);
    ui.ok("ui.layout", json!({ "hand": "left" }));
    ui.settle();
    check(&mut ui, false);
    ui.press("header > Look");
    let (at, shown, view) = (
        ui.rect("header > Look"),
        ui.rect("look"),
        ui.rect("viewport"),
    );
    assert!(
        shown[1] >= at[3] - 1.0 && shown[0] >= view[0] - 0.5 && shown[2] <= view[2] + 0.5,
        "Look's options {shown:?} under it {at:?}, inside the view {view:?}"
    );
}

/// Section 2's A dropdown panel: a menu whose set has a slider (Tone) opens as a panel under its
/// title, stays open after a change, and shuts on Escape or a press outside.
#[test]
fn a_dropdown_panel_stays_open_after_a_change_and_closes_on_escape() {
    let mut ui = Ui::new();
    with_header(&mut ui, &["file", "tone"], &[]);
    open(&mut ui, "Tone");
    let roles: Vec<Value> = ui
        .widgets("menu")
        .iter()
        .map(|w| w["role"].clone())
        .collect();
    assert!(
        roles.contains(&json!("slider")),
        "Exposure is a slider: {roles:?}"
    );
    ui.press("menu > AgX");
    assert_eq!(ui.state()["view.transform"], "agx", "the change ran");
    assert_eq!(
        shown_menu(&mut ui).as_deref(),
        Some("Tone"),
        "and the panel stays open"
    );
    ui.press("menu > Standard");
    assert_eq!(ui.state()["view.transform"], "standard");
    assert_eq!(
        shown_menu(&mut ui).as_deref(),
        Some("Tone"),
        "after each change"
    );
    ui.tap("Escape", Modifiers::default());
    assert_eq!(shown_menu(&mut ui), None, "Escape shuts it");
    open(&mut ui, "Tone");
    let [x, y] = ui.middle();
    ui.tap_at(x, y);
    assert_eq!(shown_menu(&mut ui), None, "a press outside shuts it");
}

/// Section 2's As data and section 13: `ui.menu` opens a menu by its set's name and shuts it
/// with `false`, so a key or an agent can; `ui.describe` lists the open menu as a panel titled
/// with its name; a name that is no menu of the header is refused.
#[test]
fn ui_menu_opens_a_menu_and_describe_lists_it_as_a_panel_titled_with_its_name() {
    let mut ui = Ui::new();
    let info = ui.ok("ui.menu", json!({}));
    assert_eq!(
        (&info["open"], &info["menus"]),
        (
            &Value::Null,
            &json!([
                "file",
                "add",
                "part",
                "mesh",
                "mask",
                "visible",
                "groups",
                "deform",
                "view-menu",
                "window"
            ])
        ),
        "{info}"
    );
    let info = ui.ok("ui.menu", json!({ "open": "file" }));
    assert_eq!(info["open"], "file", "{info}");
    ui.settle();
    let menu = panel(&mut ui, "menu").unwrap_or_else(|| panic!("an open menu is a panel"));
    assert_eq!(menu["title"], "File", "{menu}");
    let rows = listed(&ui.describe(), "menu", "widgets");
    assert!(
        rows.iter().any(|w| w["label"] == "Save"),
        "its items: {rows:?}"
    );
    let info = ui.ok("ui.menu", json!({ "open": false }));
    assert_eq!(info["open"], Value::Null, "{info}");
    ui.settle();
    assert_eq!(shown_menu(&mut ui), None, "shut");
    let (code, message) = ui.err("ui.menu", json!({ "open": "brush" }));
    assert_eq!(code, ErrorCode::NotFound, "{message}");
    assert!(
        message.contains("file"),
        "it names the menus there are: {message}"
    );
}

/// Section 4 as amended for H5 (items 1 and 3): the strip's buttons, then Look, sit at the
/// header's pen end in that order, each with options a section of the header, shut; there is no
/// strip column, so the 3D view reaches the scene tray.
#[test]
fn the_view_buttons_sit_at_the_pen_end_in_order_and_the_view_reaches_the_scene_tray() {
    let mut ui = Ui::new();
    let (titles, ends) = titles_and_ends(&mut ui);
    let wide = [&VIEW_BUTTONS[..], &["On", "X", "Y", "Z", "▼"]].concat();
    assert_eq!(left_to_right(&ends), wide, "{ends:?}");
    let menus_right = titles.iter().map(|(_, r)| r[2]).fold(f64::MIN, f64::max);
    let menus_foot = titles.iter().map(|(_, r)| r[3]).fold(f64::MIN, f64::max);
    // Right of the titles on their row, or on a row under them where the bar is too narrow for
    // both, as it is at the rig's 1800 points with ten menus (as amended for H7, item 1.2).
    assert!(
        ends.iter()
            .all(|(_, r)| r[0] >= menus_right || r[1] >= menus_foot),
        "at the pen end: {ends:?}"
    );
    let header = panel(&mut ui, "header").unwrap_or_default();
    let sections = header["sections"].as_array().cloned().unwrap_or_default();
    let shut: Vec<(Value, Value)> = (sections.iter())
        .map(|s| (s["id"].clone(), s["folded"].clone()))
        .collect();
    let want = [
        "frame",
        "wire",
        "floor",
        "isolate",
        "look",
        "symmetry-kinds",
    ]
    .map(|id| (json!(id), json!(true)));
    assert_eq!(shut, want, "a section per button with options, shut");
    assert!(panel(&mut ui, "strip").is_none(), "no strip column");
    let view = rect(&panel(&mut ui, "viewport").unwrap_or_default()["rect"]);
    let tray = rect(&panel(&mut ui, "tray").unwrap_or_default()["rect"]);
    // The tray's rectangle is inside its margin and divider.
    assert!(
        (0.0..=10.0).contains(&(tray[0] - view[2])),
        "the view {view:?} reaches the scene tray {tray:?}"
    );
}

/// Item 9: in a window 400 points wide a part's row is wider than the scene tray, and is clipped
/// at its edge; the header and the 3D view stay beside the tray, with no strip to cover it.
#[test]
fn a_row_wider_than_the_scene_tray_leaves_the_header_and_the_view_beside_it() {
    let mut ui = Ui::new();
    ui.host.lock().resize([800, 500], 2.0);
    ui.ok("primitive.sphere", json!({}));
    ui.settle();
    let tray = ui.rect("tray");
    for id in ["header", "viewport"] {
        let r = ui.rect(id);
        assert!(r[2] <= tray[0], "the {id} {r:?} beside the tray {tray:?}");
    }
}

/// Section 4: every view button keeps the strip's command and key, and has a hint (rule 8);
/// Look has one too.
#[test]
fn every_view_button_keeps_the_strips_command_and_key() {
    let mut ui = Ui::new();
    let want = [
        ("Frame", "view.frame", json!({}), json!("F")),
        (
            "Perspective",
            "view.lens",
            json!({ "projection": "toggle" }),
            json!("P"),
        ),
        (
            "Wire",
            "view.look",
            json!({ "flip_wireframe": true }),
            json!("Shift+F"),
        ),
        (
            "Floor",
            "view.display",
            json!({ "floor": true }),
            json!("Shift+P"),
        ),
        ("Lock", "view.lock", json!({}), json!("V L")),
        (
            "Isolate",
            "part.isolate",
            json!({ "parts": ["@active"] }),
            Value::Null,
        ),
    ];
    let widgets = ui.widgets("header");
    let find = |label: &str| {
        let found = widgets.iter().find(|w| w["label"] == label);
        found.unwrap_or_else(|| panic!("the header shows {label}: {widgets:?}"))
    };
    for (label, command, args, key) in want {
        let w = find(label);
        let step = &w["command"];
        assert_eq!(
            (&step["command"], &step["args"], &w["key"]),
            (&json!(command), &args, &key),
            "{w}"
        );
        assert!(w["hint"].is_string(), "{label} has a hint: {w}");
    }
    assert!(find("Look")["hint"].is_string(), "Look has a hint");
}

/// Amended for H5, item 3: a tap on Look opens its options as a panel under it, inside the 3D
/// view, not a menu; a change there keeps them open; Escape shuts them, and so does a press
/// outside, which does nothing else: on the model it starts no stroke, on another view button
/// it runs nothing.
#[test]
fn a_view_buttons_options_open_under_it_and_stay_open_after_a_change_until_escape_or_a_press_outside()
 {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let opened = |ui: &mut Ui| {
        ui.press("header > Look");
        panel(ui, "look").unwrap_or_else(|| panic!("a tap on Look opens its options"))
    };
    let look = opened(&mut ui);
    assert_eq!(look["title"], "Look", "{look}");
    let (at, shown, view) = (
        ui.rect("header > Look"),
        rect(&look["rect"]),
        ui.rect("viewport"),
    );
    assert!(
        shown[1] >= at[3] - 1.0,
        "under its button: {shown:?} {at:?}"
    );
    let inside = shown[0] >= view[0] - 0.5 && shown[2] <= view[2] + 0.5;
    assert!(
        inside && shown[3] <= view[3] + 0.5,
        "inside the view {view:?}: {shown:?}"
    );
    let info = ui.ok("ui.menu", json!({}));
    assert_eq!(info["open"], Value::Null, "not a menu: {info}");
    ui.press("look > AgX");
    assert_eq!(ui.state()["view.transform"], "agx", "the change ran");
    assert!(panel(&mut ui, "look").is_some(), "and they stay open");
    ui.tap("Escape", Modifiers::default());
    assert!(panel(&mut ui, "look").is_none(), "Escape shuts them");
    let undos = |ui: &mut Ui| ui.state()["history.undos"].clone();
    let before = undos(&mut ui);
    opened(&mut ui);
    let [x, y] = ui.middle();
    drag(&mut ui, x - 30.0, y);
    assert!(
        panel(&mut ui, "look").is_none(),
        "a press outside shuts them"
    );
    assert_eq!(undos(&mut ui), before, "and starts no stroke");
    opened(&mut ui);
    let lens = ui.state()["view.projection"].clone();
    ui.press("header > Perspective");
    assert!(panel(&mut ui, "look").is_none(), "Perspective shuts them");
    assert_eq!(ui.state()["view.projection"], lens, "and runs nothing");
}

/// The Reference View's bar: its widgets on the first 40 points of the `reference` panel,
/// left to right.
fn reference_bar(ui: &mut Ui) -> Shown {
    let widgets = ui.widgets("reference");
    let at: Shown = (widgets.iter())
        .map(|w| {
            (
                w["label"].as_str().unwrap_or_default().into(),
                rect(&w["rect"]),
            )
        })
        .collect();
    let top = at.iter().map(|(_, r)| r[1]).fold(f64::MAX, f64::min);
    let mut bar: Shown = at.into_iter().filter(|(_, r)| r[3] <= top + 40.5).collect();
    bar.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
    bar
}

/// The Reference View's bar, in its order.
pub(crate) const REFERENCE_BAR: [&str; 10] = [
    "Follow", "Turn", "Zoom", "Slide", "Align", "Frame", "Wire", "Look", "Next", "Close",
];

/// Amended for H5, item 7: the docked Reference View's bar is one row in the header's style,
/// Follow and its three, Align, then Frame, Wire, Look, Next and Close each an icon alone, no
/// wider than tall.
#[test]
fn the_reference_views_bar_is_one_row_in_the_headers_order_with_icons() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    ui.ok("reference.open", json!({ "scene": true }));
    ui.settle();
    let bar = reference_bar(&mut ui);
    assert_eq!(labels(&bar), REFERENCE_BAR, "{bar:?}");
    for (label, r) in &bar[5..] {
        assert!(
            r[2] - r[0] <= r[3] - r[1] + 2.0,
            "{label} is an icon alone: {r:?}"
        );
    }
}

/// Amended for H5, item 8: the floating Reference window is at least as wide as its bar on one
/// row, though the layout asks for a narrower one.
#[test]
fn the_floating_reference_window_holds_its_bar_on_one_row() {
    let mut ui = Ui::new();
    {
        let mut studio = ui.host.lock();
        let reference = &mut studio.shell.directions.list[0].reference;
        reference.place = RefPlace::Float;
        reference.width = 120.0;
    }
    ui.ok("primitive.sphere", json!({}));
    ui.ok("reference.open", json!({ "scene": true }));
    ui.settle();
    ui.settle();
    let bar = reference_bar(&mut ui);
    assert_eq!(labels(&bar), REFERENCE_BAR, "{bar:?}");
}
