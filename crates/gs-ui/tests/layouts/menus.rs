//! The Add, Part, Mesh, Mask, Visible, Groups, View and Window menus in the 3D view's header
//! (decision 0060, section 3, as amended for H3 and H4): each opens from its title and lists its
//! groups and items in order, with their keys at the right; an item runs its command once and
//! shuts the menu; Voxel mode shows the active part's switch lit; an item whose command cannot
//! run now is gray and its hint says why, and so is each one-part row of Part in an empty scene;
//! Mirror and weld is one row of three axes, and an axis of any such row runs the axis pressed;
//! and the scene tray no longer shows what moved.

use gs_protocol::Step;
use serde_json::{Value, json};

use crate::header::{item, open, shown_menu};
use crate::rig::Ui;
use crate::shelf::rect;

/// Each menu by its title, and what it shows top to bottom: its groups' headings, its rows by
/// their short labels, and each key right after its row, or a line's keys once after its last
/// button.
const MENUS: [(&str, &[&str]); 8] = [
    ("Add", &["Sphere", "Cube", "Cylinder"]),
    (
        "Part",
        &[
            "Duplicate",
            "Ctrl+Shift+D",
            "Mirror copy",
            "X",
            "Y",
            "Z",
            "Order",
            "Rename…",
            "Move up",
            "Move down",
            "Show all",
            "Split",
            "Shells",
            "Groups",
            "Mask",
            "Hidden",
            "Extract",
            "Merge",
            "Down",
            "Visible",
            "Remesh visible",
            "Delete",
            "This part",
            "Others",
            "Hidden",
            "All",
        ],
    ),
    (
        "Mesh",
        &[
            "Remesh",
            "Voxel Remesh",
            "Ctrl+R",
            "Voxel mode",
            "Quad Remesh…",
            "Decimate…",
            "Modify",
            "Mirror and weld",
            "X",
            "Y",
            "Z",
            "Project Detail…",
        ],
    ),
    (
        "Mask",
        &[
            "All",
            "Ctrl+A",
            "Invert",
            "Ctrl+I",
            "Clear",
            "Alt+M",
            "Filter",
            "Grow",
            "Shrink",
            "Blur",
            "Sharpen",
            "Boost",
            "Dilute",
            "Normalize",
            "Make",
            "From cavity",
            "From features",
        ],
    ),
    (
        "Visible",
        &[
            "Hide",
            "Masked",
            "Unmasked",
            "Show",
            "Unmasked",
            "All",
            "Area",
            "Grow",
            "Ctrl+Shift+X",
            "Shrink",
            "Ctrl+Shift+S",
            "Invert",
            "Whole",
            "Shells",
            "Ctrl+Shift+A",
            "Groups",
            "Ctrl+Shift+Q",
        ],
    ),
    (
        "Groups",
        &[
            "Make from",
            "Shells",
            "Surface angle",
            "Visible faces",
            "Facing the camera",
            "Mask",
            "Mask and clear it",
            "Ctrl+W",
            "Edit",
            "Visible into one…",
            "Merge…",
        ],
    ),
    (
        "View",
        &[
            "Level",
            "V H",
            "Look along",
            "X",
            "Y",
            "Z",
            "V X, Y, Z",
            "Iso",
            "V I",
            "Camera",
            "Undo move",
            "Alt+Z",
            "Redo move",
            "Shift+Alt+Z",
            "Reference",
            "From a file…",
            "This scene",
            "From parts…",
        ],
    ),
    (
        "Window",
        &[
            "Settings",
            "Hand",
            "Left",
            "Right",
            "Arrangement",
            "Show",
            "1",
            "2",
            "3",
            "Ctrl+1, 2, 3",
            "Save",
            "1",
            "2",
            "3",
            "Ctrl+Shift+1, 2, 3",
            "Next",
            "Previous",
            "Restore",
            "Panels",
            "Left tray",
            "Right tray",
            "All panels",
            "Tab",
            "One section open",
        ],
    ),
];

/// The keys a key's label names: itself, or a line's keys written once, the start they share and
/// then the rest of each ("V X, Y, Z" names V X, V Y and V Z).
fn keys_of(label: &str) -> Vec<String> {
    let mut parts = label.split(", ");
    let first = parts.next().unwrap_or_default();
    let start = &first[..first.rfind(['+', ' ']).map_or(0, |i| i + 1)];
    let rest = parts.map(|p| format!("{start}{p}"));
    std::iter::once(first.to_string()).chain(rest).collect()
}

/// The open menu's widget labelled `label`, with where it shows.
fn at(ui: &mut Ui, label: &str) -> [f64; 4] {
    let widgets = ui.widgets("menu");
    let found = widgets.iter().find(|w| w["label"] == label);
    rect(&found.unwrap_or_else(|| panic!("the menu shows {label}: {widgets:?}"))["rect"])
}

/// Section 2's A menu and section 3 (section 17's order): the header's titles are File, the
/// eight menus and Deform (a panel, D1) between Groups and View, and each menu opens from its
/// title with its headings, rows and keys in order, each key level with its row in the menu's
/// right half.
#[test]
fn each_menu_opens_from_its_title_and_lists_its_groups_and_items_in_order_keys_at_the_right() {
    let mut ui = Ui::new();
    let titles: Vec<Value> = (ui.widgets("header").into_iter())
        .filter(|w| w["command"]["command"] == "ui.menu")
        .map(|w| w["label"].clone())
        .collect();
    let all = [
        "File", "Add", "Part", "Mesh", "Mask", "Visible", "Groups", "Deform", "View", "Window",
    ];
    assert_eq!(titles, all);
    for (title, listed) in MENUS {
        open(&mut ui, title);
        let widgets = ui.widgets("menu");
        let labels: Vec<&str> = (widgets.iter())
            .map(|w| w["label"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(labels, listed, "{title}");
        let menu = ui.rect("menu");
        for (i, row) in widgets.iter().enumerate() {
            let Some(key) = row["key"].as_str().filter(|_| row["role"] != "label") else {
                continue;
            };
            // A row's key is the label after it; a line's keys show once, after its last button.
            let shown = widgets[i + 1..].iter().find(|w| w["role"] == "label");
            let shown = shown.unwrap_or_else(|| panic!("{title}: {row} shows its key"));
            let label = shown["label"].as_str().unwrap_or_default();
            assert!(
                keys_of(label).contains(&key.to_string()),
                "{title}: {label} names {key}"
            );
            let (r, k) = (rect(&row["rect"]), rect(&shown["rect"]));
            let level = k[1] >= r[1] - 0.5 && k[3] <= r[3] + 0.5;
            assert!(
                level && k[2] <= menu[2] + 0.5 && k[0] > f64::midpoint(menu[0], menu[2]),
                "{title}: {label} at the right of {}, {r:?}: {k:?} in {menu:?}",
                row["label"]
            );
        }
    }
}

/// Opens the menu `title` and taps its row `label`, which shows `want` as its step: the step
/// runs once, as the human's, and the menu shuts.
fn pick(ui: &mut Ui, title: &str, label: &str, want: &Step) {
    open(ui, title);
    let row = item(ui, label);
    let shown: Step = serde_json::from_value(row["command"].clone())
        .unwrap_or_else(|e| panic!("{title} > {label} runs something: {e}: {row}"));
    assert_eq!(&shown, want, "{title} > {label}");
    let before = ui.logged(&want.command).len();
    ui.press(&format!("menu > {label}"));
    assert_eq!(shown_menu(ui), None, "a pick shuts {title}");
    let lines = ui.logged(&want.command);
    let ran: Vec<(&Value, &Value)> = (lines.iter().skip(before))
        .map(|l| (&l["commands"], &l["role"]))
        .collect();
    let once = (&json!([want.command]), &json!("human"));
    assert_eq!(ran, [once], "{title} > {label} ran once: {lines:?}");
}

/// A step of `command` with `args`.
fn step(command: &str, args: &Value) -> Step {
    serde_json::from_value(json!({ "command": command, "args": args }))
        .unwrap_or_else(|e| panic!("{command}: {e}"))
}

/// Section 2's Opening: a pick runs and shuts the menu. One item of each menu: Voxel Remesh at
/// the cell size the part keeps (as the tray button did, until H2's adjust panel), a Mask filter,
/// a Visible area step and a Groups maker.
#[test]
fn an_item_runs_its_command_once_and_shuts_the_menu() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.ok("mesh.voxel_remesh", json!({ "resolution": 24 }));
    let remesh = step("mesh.voxel_remesh", &json!({}));
    pick(&mut ui, "Mesh", "Voxel Remesh", &remesh);
    ui.ok("mask.set", json!({ "value": 1 }));
    let blur = step("mask.filter", &json!({ "op": "blur" }));
    pick(&mut ui, "Mask", "Blur", &blur);
    let hide = json!({ "select": { "by": "faces", "indexes": [0, 1, 2, 3] } });
    ui.ok("visibility.hide", hide);
    pick(
        &mut ui,
        "Visible",
        "Grow",
        &step("visibility.grow", &json!({})),
    );
    let shells = step("group.make", &json!({ "maker": { "by": "shells" } }));
    pick(&mut ui, "Groups", "Shells", &shells);
}

/// Section 2's Opening, for the menus H4 added: Add's Sphere with no arguments, its settings
/// left to the adjust panel (`A1`), one Part split, one View look and one Window switch.
#[test]
fn an_item_of_add_part_view_and_window_runs_its_command_once_and_shuts_the_menu() {
    let mut ui = Ui::new();
    pick(
        &mut ui,
        "Add",
        "Sphere",
        &step("primitive.sphere", &json!({})),
    );
    // Split refuses a part with detail levels, so the split runs on a level-free part.
    ui.ok("primitive.sphere", json!({}));
    let hide = json!({ "select": { "by": "faces", "indexes": [0, 1, 2, 3] } });
    ui.ok("visibility.hide", hide);
    let hidden = step("part.split", &json!({ "by": "hidden" }));
    pick(&mut ui, "Part", "Hidden", &hidden);
    let iso = step("view.align", &json!({ "to": "iso" }));
    pick(&mut ui, "View", "Iso", &iso);
    // Window's commands change the window, not the scene, so the log keeps none: a switch
    // picked once shows in the state. Lit, its row runs it off.
    open(&mut ui, "Window");
    let row = item(&mut ui, "One section open");
    let shown: Option<Step> = serde_json::from_value(row["command"].clone()).ok();
    let off = step("ui.layout", &json!({ "one_open": false }));
    assert_eq!(shown, Some(off), "Window > One section open");
    ui.press("menu > One section open");
    assert_eq!(shown_menu(&mut ui), None, "a pick shuts Window");
    assert_eq!(ui.state()["layout.one_open"], json!(false), "it ran once");
}

/// Section 3's `[X][Y][Z]` rows (section 8's `line`): a tap on an axis runs that axis, Part's
/// Mirror copy Y and View's Look along Z, and a line with keys shows them once at its right.
#[test]
fn an_axis_row_runs_the_axis_pressed() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let y = step("part.mirror_copy", &json!({ "axis": "y" }));
    pick(&mut ui, "Part", "Y", &y);
    let z = step("view.align", &json!({ "to": "front", "toggle": true }));
    pick(&mut ui, "View", "Z", &z);
    open(&mut ui, "View");
    let row = ["Look along", "X", "Y", "Z", "V X, Y, Z"].map(|label| at(&mut ui, label));
    let centre = |r: [f64; 4]| f64::midpoint(r[1], r[3]);
    for (r, next) in row.iter().zip(&row[1..]) {
        assert!(r[2] <= next[0] + 0.5, "{row:?} reads left to right");
        assert!(
            (centre(*r) - centre(*next)).abs() <= 2.0,
            "{row:?} on one row"
        );
    }
}

/// Section 3's Part menu with no active part, as amended for H4 (item 4): decision 0033 keeps a
/// part active while the scene has one, so none is active only in an empty scene, where each
/// one-part row is gray with the engine's reason, as the scene tray's were, and a tap runs
/// nothing and opens no form asking which part. With a part they run on the active one.
#[test]
fn the_part_menu_with_no_part_is_gray_with_the_engines_reason_and_asks_nothing() {
    let mut ui = Ui::new();
    let why = json!("the scene has no parts");
    open(&mut ui, "Part");
    for label in [
        "Duplicate",
        "X",
        "Move up",
        "Move down",
        "Shells",
        "Extract",
        "Down",
        "This part",
    ] {
        let row = item(&mut ui, label);
        assert_eq!(
            (&row["enabled"], &row["reason"], &row["hint"]),
            (&json!(false), &why, &why),
            "Part > {label}: {row}"
        );
    }
    ui.press("menu > Duplicate");
    assert_eq!(ui.logged("part."), Vec::<Value>::new(), "nothing ran");
    assert!(ui.widgets("palette").is_empty(), "no form asks which part");
    ui.ok("primitive.sphere", json!({}));
    ui.settle();
    open(&mut ui, "Part");
    let duplicate = item(&mut ui, "Duplicate");
    assert_eq!(duplicate["enabled"], true, "{duplicate}");
}

/// Section 2's lit switch: Mesh > Voxel mode shows the active part's switch (`parts.voxel_mode`),
/// unlit and gray with no part, lit while the switch is on, and a pick flips it.
#[test]
fn voxel_mode_shows_the_active_parts_switch_lit() {
    let mut ui = Ui::new();
    open(&mut ui, "Mesh");
    let none = item(&mut ui, "Voxel mode");
    let lit_and_on = |w: &Value| (w["selected"].clone(), w["enabled"].clone());
    assert_eq!(lit_and_on(&none), (json!(false), json!(false)), "{none}");
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.ok("mesh.voxel_remesh", json!({ "resolution": 24 }));
    ui.settle();
    let off = item(&mut ui, "Voxel mode");
    assert_eq!(off["command"]["command"], "mesh.voxel_mode", "{off}");
    assert_eq!(lit_and_on(&off), (json!(false), json!(true)), "{off}");
    ui.ok("mesh.voxel_mode", json!({ "on": true }));
    ui.settle();
    let on = item(&mut ui, "Voxel mode");
    assert_eq!(
        lit_and_on(&on),
        (json!(true), json!(true)),
        "lit while on: {on}"
    );
    ui.press("menu > Voxel mode");
    assert_eq!(shown_menu(&mut ui), None, "a pick shuts the menu");
    assert_eq!(
        ui.state()["parts.voxel_mode"],
        json!(false),
        "and flips it off"
    );
    open(&mut ui, "Mesh");
    assert_eq!(item(&mut ui, "Voxel mode")["selected"], json!(false));
}

/// Section 2's A menu and rule 7: in an empty scene an item of each menu, and an axis of Mirror
/// and weld, is gray with the engine's reason as its hint; with a part it can run, and its hint
/// says what it does (section 11).
#[test]
fn an_item_whose_command_cannot_run_now_is_gray_and_its_hint_says_why() {
    let mut ui = Ui::new();
    let why = json!("the scene has no parts");
    for (title, label) in [
        ("Mesh", "Voxel Remesh"),
        ("Mesh", "X"),
        ("Mask", "Invert"),
        ("Visible", "Grow"),
        ("Groups", "Shells"),
    ] {
        open(&mut ui, title);
        let row = item(&mut ui, label);
        assert_eq!(
            (&row["enabled"], &row["reason"], &row["hint"]),
            (&json!(false), &why, &why),
            "{title} > {label}: {row}"
        );
    }
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    ui.settle();
    open(&mut ui, "Mask");
    let invert = item(&mut ui, "Invert");
    assert_eq!(invert["enabled"], true, "{invert}");
    let hint = invert["hint"].as_str().unwrap_or_default();
    assert!(hint.starts_with("Masked becomes unmasked"), "{invert}");
    // Invert and Clear take the hidden points too, as their keys do (`R9`).
    let clear = item(&mut ui, "Clear");
    let args = |row: &Value| row["command"]["args"].clone();
    assert_eq!(args(&invert), json!({ "scope": "all" }), "{invert}");
    assert_eq!(args(&clear), json!({ "value": 0, "scope": "all" }));
}

/// Section 3's `Mirror and weld [X][Y][Z]` (section 8's `line`, in a menu as amended for H3): its
/// label and the three axes read left to right on one row inside the menu, Project Detail… under
/// it, and a tap on an axis runs `mesh.symmetrize` on it.
#[test]
fn mirror_and_weld_is_one_row_of_three_axes() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    open(&mut ui, "Mesh");
    let row = ["Mirror and weld", "X", "Y", "Z"].map(|label| at(&mut ui, label));
    let centre = |r: [f64; 4]| f64::midpoint(r[1], r[3]);
    for (r, next) in row.iter().zip(&row[1..]) {
        assert!(r[2] <= next[0] + 0.5, "{row:?} reads left to right");
        assert!(
            (centre(*r) - centre(*next)).abs() <= 2.0,
            "{row:?} on one row"
        );
    }
    let menu = ui.rect("menu");
    assert!(row[3][2] <= menu[2], "{row:?} fits in the menu {menu:?}");
    let under = at(&mut ui, "Project Detail…");
    assert!(under[1] >= row[1][3] - 0.5, "{under:?} under {row:?}");
    let y = step("mesh.symmetrize", &json!({ "axis": "y" }));
    pick(&mut ui, "Mesh", "Y", &y);
}

/// Section 7: Masking and Face Groups are no longer scene tray sections, and Detail (Geometry
/// until S1) keeps its detail levels and Live Detail but none of the operations the Mesh menu
/// took.
#[test]
fn the_scene_tray_no_longer_shows_the_moved_sets() {
    let mut ui = Ui::new();
    let sections = ui.open_only(&["detail"]);
    for gone in ["masking", "face-groups"] {
        assert!(!sections.iter().any(|s| s == gone), "{gone}: {sections:?}");
    }
    let labels = ui.labels("tray");
    let shows = |label: &str| labels.iter().any(|l| l == label);
    for gone in [
        "Remesh",
        "Voxel Remesh",
        "Voxel Remesh to a resolution",
        "Voxel mode",
        "Quad Remesh",
        "Decimate",
        "Modify",
        "Mirror and weld X",
        "Project Detail",
    ] {
        assert!(!shows(gone), "{gone} left the tray: {labels:?}");
    }
    for kept in ["‹", "Subdivide", "···", "Live"] {
        assert!(shows(kept), "{kept} stays: {labels:?}");
    }
}

/// Section 7 and section 15, as amended for H4 (items 6 and 8): New, Reference and Interface are
/// no longer scene tray sections, and Parts keeps its list but none of the operations the Part
/// menu took, nor `part.add`'s buttons; Rows shown went to Settings > Interface (S1).
#[test]
fn the_scene_tray_no_longer_shows_new_reference_interface_or_the_parts_operations() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    let sections = ui.open_only(&["parts"]);
    let labels = ui.labels("tray");
    let shows = |label: &str| labels.iter().any(|l| l == label);
    for gone in [
        "Add sphere",
        "Add cube",
        "Duplicate",
        "Mirror copy X",
        "Rename",
        "Move up",
        "Show all parts",
        "Split into shells",
        "Extract",
        "Merge down",
        "Remesh visible",
        "Delete",
        "Delete all",
    ] {
        assert!(!shows(gone), "{gone} left the tray: {labels:?}");
    }
    assert!(shows("sphere"), "the list stays: {labels:?}");
    // Keys is Settings' Keys page since R2 (section 7); Deform a header dropdown since D1.
    assert_eq!(sections, ["parts", "detail", "history"]);
}
