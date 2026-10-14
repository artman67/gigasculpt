//! The header fits a narrow view (decision 0060 as amended for H7): when the menus' titles do not
//! fit a row they fold into one button, Menus, so the bar takes at most two rows at 640, 800 and
//! 1280 points. Through it every menu lists what it lists unfolded, each item running the same
//! step; a menu opened through it shuts on Escape or a press outside, which does nothing else; a
//! key bound to `ui.menu` opens a menu through it, and `ui.describe` names each part. The
//! Reference View's bar wraps and never folds, and the left hand mirrors the bar.

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::header::{REFERENCE_BAR, VIEW_BUTTONS, drag, items, open, panel, shown_menu};
use crate::rig::{Ui, held};
use crate::shelf::rect;

/// The header's menus by title, in order, Deform's panel among them (D1).
const TITLES: [&str; 10] = [
    "File", "Add", "Part", "Mesh", "Mask", "Visible", "Groups", "Deform", "View", "Window",
];

/// The window resized to `size` points, and the frames that lay it out again.
fn sized(ui: &mut Ui, size: [u32; 2]) {
    ui.host.lock().resize(size, 1.0);
    ui.settle();
    ui.settle();
}

/// The header's rows: 40 points each, a 32-point control and the 8 above or between.
fn rows(ui: &mut Ui) -> f64 {
    let bar = ui.rect("header");
    ((bar[3] - bar[1]) / 40.0).round()
}

/// The open menu's rows, top to bottom: each one's label, step and key.
fn rows_of(ui: &mut Ui) -> Vec<(Value, Value, Value)> {
    let rows = items(ui).into_iter();
    rows.map(|w| (w["label"].clone(), w["command"].clone(), w["key"].clone()))
        .collect()
}

/// Opens the menu `title` through the fold: a tap on Menus, unless its list shows, then one on
/// the menu's row there.
fn through_fold(ui: &mut Ui, title: &str) {
    if panel(ui, "menus").is_none() {
        ui.press("header > Menus");
    }
    ui.press(&format!("menus > {title}"));
    assert_eq!(shown_menu(ui).as_deref(), Some(title), "{title} opened");
}

/// The row a widget at `r` sits on, by its middle: a line's label is shorter than its buttons.
fn row(r: &[f64; 4]) -> f64 {
    ((r[1] + r[3]) / 40.0).round()
}

/// Widgets with where they show, in reading order: by row, then left to right.
fn reading(widgets: &[Value]) -> Vec<(String, [f64; 4])> {
    let mut shown: Vec<(String, [f64; 4])> = (widgets.iter())
        .map(|w| {
            (
                w["label"].as_str().unwrap_or_default().into(),
                rect(&w["rect"]),
            )
        })
        .collect();
    shown.sort_by(|a, b| {
        row(&a.1)
            .total_cmp(&row(&b.1))
            .then(a.1[0].total_cmp(&b.1[0]))
    });
    shown
}

/// Item 1: the header takes at most two rows at 640 by 480, 800 by 600 and 1280 by 800 points,
/// and one at 1920 by 1080, every view button showing at each.
#[test]
fn the_header_takes_at_most_two_rows_at_640_800_and_1280_points() {
    let mut ui = Ui::new();
    let sizes = [
        ([640, 480], 2.0),
        ([800, 600], 2.0),
        ([1280, 800], 2.0),
        ([1920, 1080], 1.0),
    ];
    for (size, most) in sizes {
        sized(&mut ui, size);
        let taken = rows(&mut ui);
        assert!(taken <= most, "{size:?}: the header takes {taken} rows");
        let shown = ui.labels("header");
        for button in VIEW_BUTTONS {
            assert!(
                shown.iter().any(|l| l == button),
                "{size:?}: {button} shows: {shown:?}"
            );
        }
    }
}

/// Items 1, 3 and 6: folded, the bar has no titles but the button Menus, whose list names the
/// ten menus in order; through it each menu lists the rows, steps and keys it lists unfolded,
/// in the same order, and opens beside its row; a pick runs once, as the human's, and shuts the
/// menu and the list.
#[test]
fn the_fold_reaches_every_item_the_menus_do_in_order_each_running_the_same_step() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.ok("mask.set", json!({ "value": 1 }));
    let mut unfolded = Vec::new();
    for title in TITLES {
        open(&mut ui, title);
        unfolded.push(rows_of(&mut ui));
    }
    ui.tap("Escape", Modifiers::default());
    // As narrow as a 640-point window, and taller than the rig's, so every menu's rows show:
    // Deform's panel, beside its row low in the list, would scroll in 1100 points.
    sized(&mut ui, [640, 1400]);
    let header = ui.widgets("header");
    let titled = (header.iter()).filter(|w| w["command"]["command"] == "ui.menu");
    assert_eq!(titled.count(), 0, "no title in the bar: {header:?}");
    ui.press("header > Menus");
    assert_eq!(
        ui.labels("menus"),
        TITLES,
        "the list names the menus in order"
    );
    for (title, want) in TITLES.iter().zip(&unfolded) {
        through_fold(&mut ui, title);
        assert_eq!(&rows_of(&mut ui), want, "{title} through the fold");
        let (row, menu) = (ui.rect(&format!("menus > {title}")), ui.rect("menu"));
        assert!(
            menu[0] >= row[2] - 1.0,
            "{title} {menu:?} beside its row {row:?}"
        );
    }
    ui.tap("Escape", Modifiers::default());
    through_fold(&mut ui, "Mask");
    let before = ui.logged("mask.filter").len();
    ui.press("menu > Blur");
    assert_eq!(shown_menu(&mut ui), None, "a pick shuts the menu");
    assert!(panel(&mut ui, "menus").is_none(), "and the list");
    let ran = ui.logged("mask.filter");
    let ran: Vec<&Value> = ran[before..].iter().map(|l| &l["role"]).collect();
    assert_eq!(ran, [&json!("human")], "Blur ran once");
}

/// Item 3: Escape shuts a menu opened through the fold and the list; a press outside the list
/// shuts it and starts no stroke on the model, where the same drag with nothing open strokes; a
/// press on a view button while a menu is open through it shuts both and runs nothing; a tap on
/// Menus while its list shows shuts it.
#[test]
fn a_menu_through_the_fold_shuts_on_escape_and_on_a_press_outside_which_does_nothing_else() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    sized(&mut ui, [640, 480]);
    let shut = |ui: &mut Ui| shown_menu(ui).is_none() && panel(ui, "menus").is_none();
    let undos = |ui: &mut Ui| ui.state()["history.undos"].clone();
    let before = undos(&mut ui);
    through_fold(&mut ui, "File");
    ui.tap("Escape", Modifiers::default());
    assert!(shut(&mut ui), "Escape shuts the menu and the list");
    ui.press("header > Menus");
    let [x, y] = ui.middle();
    drag(&mut ui, x, y);
    assert!(shut(&mut ui), "a press outside shuts the list");
    assert_eq!(undos(&mut ui), before, "and starts no stroke");
    drag(&mut ui, x, y);
    assert_ne!(
        undos(&mut ui),
        before,
        "with nothing open the same drag strokes"
    );
    through_fold(&mut ui, "Mesh");
    let lens = ui.state()["view.projection"].clone();
    ui.press("header > Perspective");
    assert!(shut(&mut ui), "a press on Perspective shuts both");
    assert_eq!(ui.state()["view.projection"], lens, "and runs nothing");
    ui.press("header > Menus");
    assert!(
        panel(&mut ui, "menus").is_some(),
        "a tap on Menus lists them"
    );
    ui.press("header > Menus");
    assert!(shut(&mut ui), "a second tap shuts the list");
}

/// Items 6 and 7: a key bound to `ui.menu` opens its menu through the fold, the list with it and
/// the menu's row lit there; `ui.describe` lists Menus in the header with its hint, lit while
/// open, the list as the panel `menus` whose rows run `ui.menu`, and the menu as the panel
/// `menu` titled with its name, which `ui.menu` reports open.
#[test]
fn a_key_bound_to_ui_menu_opens_its_menu_through_the_fold_and_describe_names_each_part() {
    let mut ui = Ui::new();
    sized(&mut ui, [640, 480]);
    let fold = |ui: &mut Ui| {
        let header = ui.widgets("header");
        let found = header.into_iter().find(|w| w["label"] == "Menus");
        found.unwrap_or_else(|| panic!("Menus in the header"))
    };
    let shut = fold(&mut ui);
    assert!(shut["hint"].is_string(), "Menus has a hint: {shut}");
    assert_eq!(shut["selected"], false, "{shut}");
    let bind = json!({
        "bind": "Ctrl+K",
        "command": "ui.menu",
        "args": { "open": "mesh" },
        "replace": true,
    });
    ui.ok("ui.keymap", bind);
    ui.tap("K", held("Ctrl"));
    ui.settle();
    assert_eq!(
        shown_menu(&mut ui).as_deref(),
        Some("Mesh"),
        "the key opened Mesh"
    );
    assert_eq!(fold(&mut ui)["selected"], true, "Menus is lit while open");
    let list = panel(&mut ui, "menus").unwrap_or_else(|| panic!("the list shows with it"));
    assert_eq!(list["title"], "Menus", "{list}");
    let mesh = ui
        .widgets("menus")
        .into_iter()
        .find(|w| w["label"] == "Mesh");
    let mesh = mesh.unwrap_or_else(|| panic!("Mesh in the list"));
    assert_eq!(mesh["command"]["command"], "ui.menu", "{mesh}");
    assert_eq!(mesh["selected"], true, "Mesh is lit in the list: {mesh}");
    assert_eq!(ui.ok("ui.menu", json!({}))["open"], "mesh");
}

/// The Reference View's bar: its widgets in reading order.
fn reference_bar(ui: &mut Ui) -> Vec<(String, [f64; 4])> {
    let widgets = ui.widgets("reference");
    let bar: Vec<Value> = (widgets.into_iter())
        .filter(|w| REFERENCE_BAR.contains(&w["label"].as_str().unwrap_or_default()))
        .collect();
    reading(&bar)
}

/// Item 5: beside a docked Reference View at 1280 by 800 points the header takes at most two
/// rows, and so does the Reference View's bar, which keeps every control in its order and never
/// scrolls. At 1920 by 1080 its bar was one row until UI-keys put Look's words after it, which
/// the layout's 560 points do not hold: two rows there too. Each size is a session of its own,
/// opened in a window that size, as a docked Reference View keeps the width it first took.
#[test]
fn beside_a_docked_reference_view_both_bars_fit_and_the_reference_bar_never_folds() {
    for (size, most) in [([1280, 800], 2), ([1920, 1080], 2)] {
        let mut ui = Ui::new();
        sized(&mut ui, size);
        ui.ok("primitive.sphere", json!({}));
        ui.ok("reference.open", json!({ "scene": true }));
        ui.settle();
        let taken = rows(&mut ui);
        assert!(taken <= 2.0, "{size:?}: the header takes {taken} rows");
        let bar = reference_bar(&mut ui);
        let labels: Vec<&str> = bar.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, REFERENCE_BAR, "{size:?}: every control, in order");
        let mut tops: Vec<f64> = bar.iter().map(|(_, r)| row(r)).collect();
        tops.dedup();
        assert!(
            tops.len() <= most,
            "{size:?}: its bar takes {} rows",
            tops.len()
        );
        let widgets = ui.widgets("reference");
        let scrolls = widgets.iter().any(|w| w["role"] == "scroll bar");
        assert!(!scrolls, "{size:?}: it never scrolls");
    }
}

/// Item 4: for the left hand Menus sits at the bar's right end and the pen end at its left, in
/// its order; the list opens under Menus, inside the view, and a menu opens beside its row
/// toward the view, to its left.
#[test]
fn the_left_hand_mirrors_the_fold() {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "hand": "left" }));
    sized(&mut ui, [640, 480]);
    let (bar, fold) = (ui.rect("header"), ui.rect("header > Menus"));
    assert!(
        bar[2] - fold[2] <= 12.0,
        "Menus {fold:?} at the right end of {bar:?}"
    );
    let widgets = ui.widgets("header");
    let ends = reading(&widgets);
    let ends: Vec<&(String, [f64; 4])> = ends.iter().filter(|(l, _)| l != "Menus").collect();
    let labels: Vec<&str> = ends.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, VIEW_BUTTONS, "the pen end keeps its order");
    for (label, at) in &ends {
        assert!(
            at[2] <= fold[0] + 0.5,
            "{label} {at:?} left of Menus {fold:?}"
        );
    }
    ui.press("header > Menus");
    let (list, view) = (ui.rect("menus"), ui.rect("viewport"));
    // The ten titles take about 340 points, more than 480 leaves under Menus: the list slides up
    // over it only that far, to the window's foot (decision 0060 as amended for H7, item 3).
    assert!(
        list[1] >= fold[3] - 1.0 || (list[3] - 480.0).abs() <= 0.5,
        "the list {list:?} under Menus {fold:?}, or at the window's foot"
    );
    let inside = list[0] >= view[0] - 0.5 && list[2] <= view[2] + 0.5;
    assert!(inside, "the list {list:?} inside the view {view:?}");
    ui.press("menus > File");
    let (row, menu) = (ui.rect("menus > File"), ui.rect("menu"));
    assert!(
        menu[2] <= row[0] + 1.0,
        "File {menu:?} to the left of its row {row:?}"
    );
}
