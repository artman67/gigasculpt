//! The brush shelf is as tall as its rows: in a window too narrow for one row its controls wrap
//! and the shelf grows to show every row whole, the trays starting below it, up to a third of
//! the window (its three rows at 640 by 480 points), past which it scrolls; wide again, it
//! shrinks back to the height the layout file gives it.
//!
//! The Workbench shelf shows a button per brush family (decision 0025, amended 2026-10-12,
//! Shelf-family), so it stays within two rows at 1800 points however many brushes a family
//! holds, and the shipped brushes take one now that Brush file is the brush's ⋯. A held press, a right click or a second tap opens the family at the pointer, where a
//! pick is one brush step; Escape or a tap outside shuts it, running nothing.

use std::collections::BTreeSet;

use gs_engine::BrushInfo;
use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::rig::{SIZE, Ui, center};

/// A rectangle as `ui.describe` reports it: min x, min y, max x, max y.
pub fn rect(v: &Value) -> [f64; 4] {
    std::array::from_fn(|i| v[i].as_f64().unwrap_or_default())
}

/// The shelf in a window `size` points wide and tall, as `ui.describe` reports it.
struct Shelf {
    rect: [f64; 4],
    /// The tops of its rows.
    rows: BTreeSet<i64>,
    /// Its widgets' heights: one cut by the shelf's edge is reported cut to what shows, and
    /// one out of sight is left out.
    heights: Vec<f64>,
    /// Where the scene tray starts.
    tray: f64,
}

fn shelf_at(ui: &mut Ui, size: [u32; 2]) -> Shelf {
    ui.host.lock().resize(size, 1.0);
    let d = ui.describe();
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let panel = |id: &str| panels.iter().find(|p| p["id"] == id).cloned();
    let shelf = panel("shelf").unwrap_or_else(|| panic!("no shelf"));
    let widgets = shelf["widgets"].as_array().cloned().unwrap_or_default();
    let rects: Vec<[f64; 4]> = widgets.iter().map(|w| rect(&w["rect"])).collect();
    Shelf {
        rect: rect(&shelf["rect"]),
        rows: rects.iter().map(|r| r[1].round() as i64).collect(),
        heights: rects.iter().map(|r| r[3] - r[1]).collect(),
        tray: panel("tray").map_or(0.0, |t| rect(&t["rect"])[1]),
    }
}

#[test]
fn the_shelf_grows_to_show_every_row_and_shrinks_back_when_they_fit_on_one() {
    let mut ui = Ui::new();
    let wide = shelf_at(&mut ui, [3600, 1100]);
    assert_eq!(
        wide.rows.len(),
        1,
        "one row in a wide window: {:?}",
        wide.rows
    );
    let whole = |s: &Shelf| s.heights == wide.heights;

    let narrow = shelf_at(&mut ui, [1600, 1100]);
    assert!(
        narrow.rows.len() > 1,
        "at 1600 points it wraps: {:?}",
        narrow.rows
    );
    assert!(
        whole(&narrow),
        "every row shows whole, a group label starting one as tall as its buttons: {:?}",
        narrow.heights
    );
    assert!(
        narrow.tray >= narrow.rect[3],
        "the scene tray starts below it"
    );

    // Too short a window for that: a third of it, and the rest scrolls.
    let tiny = shelf_at(&mut ui, [1600, 240]);
    assert!(tiny.rect[3] <= 240.0 / 3.0, "{:?}", tiny.rect);
    assert!(
        !whole(&tiny),
        "some rows are out of sight: {:?}",
        tiny.heights
    );

    let again = shelf_at(&mut ui, [3600, 1100]);
    assert_eq!(again.rect, wide.rect, "wide again, it is as tall as before");
}

/// At 640 by 480 points, the smallest window the tests size, the shelf wraps and still shows
/// every row whole, with no scroll bar: its last row was cut off at its edge.
#[test]
fn at_640_by_480_points_the_shelf_shows_every_row_whole() {
    let mut ui = Ui::new();
    let wide = shelf_at(&mut ui, [3600, 1100]);
    let small = shelf_at(&mut ui, [640, 480]);
    assert!(small.rows.len() > 1, "it wraps: {:?}", small.rows);
    let rect = small.rect;
    assert_eq!(small.heights, wide.heights, "every row whole in {rect:?}");
    assert!(
        small.tray >= small.rect[3],
        "the scene tray starts below it"
    );
}

/// 44 brushes more, by family: names from the catalog's phase 2 brushes but the two shipped
/// already, and two families of the test's own.
const MORE: [(&str, &str); 8] = [
    (
        "Build",
        "Blob, Clay Tubes, Elastic Inflate, Fixed Height, Magnify, Swell",
    ),
    (
        "Carve",
        "Clip Circle, Clip Circle From Center, Clip Rectangle, Planar, Planar Cut, Slash 1, \
         Slash 2, Slash 3, Slice Curve, Trim Adaptive, Trim Circle, Trim Curve, Trim Lasso, \
         Trim Rectangle, Trim Smooth Border",
    ),
    ("Move", "Move Through Depth, Nudge, Transpose Line"),
    (
        "Smooth",
        "Flatten, Polish, Smooth Alternate, Smooth Along Stroke, Smooth By Connections, \
         Smooth Face Groups, Smooth Like Subdivide, Smooth Peaks, Smooth Valleys",
    ),
    (
        "Mask",
        "Mask Circle, Mask Curve, Mask Lasso, Mask Rectangle, Select Lasso",
    ),
    ("Paint", "Paint Soft, Paint Spray"),
    ("Topology", "Poly Edit Brush, Quad Remesh Guide"),
    ("Insert", "Insert Cylinder, Insert Sphere"),
];

/// Lists the shipped brushes and [`MORE`] in `brush.list`'s order: by family, then by name.
fn add_more(ui: &mut Ui) {
    let mut studio = ui.host.lock();
    let brushes = &mut studio.shell.directions.brushes;
    let like = brushes[0].clone();
    for (family, names) in MORE {
        for name in names.split(", ") {
            let id = format!("brush.test-{}", name.to_lowercase().replace(' ', "-"));
            let (name, family) = (name.to_string(), family.to_string());
            brushes.push(BrushInfo {
                name,
                id,
                family,
                ..like.clone()
            });
        }
    }
    let rank = |b: &BrushInfo| MORE.iter().position(|(f, _)| *f == b.family);
    brushes.sort_by(|a, b| (rank(a), &a.name).cmp(&(rank(b), &b.name)));
}

#[test]
fn the_workbench_shelf_takes_two_rows_at_most_at_1800_points_with_44_more_brushes() {
    let mut ui = Ui::new();
    let shipped = shelf_at(&mut ui, SIZE);
    let rows = shipped.rows.len();
    // Brush file left for the brush's ⋯ in the tray (decision 0060, section 10).
    assert_eq!(rows, 1, "the shipped brushes: {rows} rows");
    add_more(&mut ui);
    ui.settle();
    let more = shelf_at(&mut ui, SIZE);
    let rows = more.rows.len();
    assert!(rows <= 2, "with 44 more: {rows} rows");
    let widgets = ui.widgets("shelf");
    assert!(
        !widgets.iter().any(|w| w["role"] == "scroll bar"),
        "every row shows: {:?}",
        more.heights
    );
    let picks = widgets
        .iter()
        .filter(|w| w["command"]["args"]["brush"].is_string());
    assert_eq!(picks.count(), MORE.len(), "a button per family");
}

/// The command log's lines but the pen's own (`pen.input`): what the interface ran.
fn ran(ui: &mut Ui) -> Vec<Value> {
    let tail = ui.ok("log.tail", json!({ "n": 500 }));
    let all = tail["entries"].as_array().cloned().unwrap_or_default();
    (all.into_iter())
        .filter(|l| l["commands"] != json!(["pen.input"]))
        .collect()
}

/// The open family, as `ui.describe` reports its panel.
fn family(ui: &mut Ui) -> Option<Value> {
    let d = ui.describe();
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    panels.into_iter().find(|p| p["id"] == "family")
}

/// A widget's label and whether it is lit.
fn shown(w: &Value) -> (Value, Value) {
    (w["label"].clone(), w["selected"].clone())
}

#[test]
fn a_family_button_opens_its_family_at_the_pointer_and_a_pick_there_is_one_brush_step() {
    let mut ui = Ui::new();
    // A favorite still shows on the shelf, after the families (W3b).
    let elastic = json!({ "command": "brush.set", "args": { "brush": "Move Elastic" } });
    ui.ok("ui.library", json!({ "pin": elastic }));
    ui.settle();
    let labels = ui.labels("shelf");
    let favorites = labels.iter().position(|l| l == "Favorites");
    let pinned = favorites.and_then(|i| labels.get(i + 1));
    assert_eq!(
        pinned.map(String::as_str),
        Some("Move Elastic"),
        "{labels:?}"
    );

    let face = ui.face("Move");
    assert_eq!(
        shown(&face),
        (json!("Move"), json!(false)),
        "its first brush"
    );
    let before = ran(&mut ui);
    let [x, y] = center(&face["rect"]);
    ui.hold_at(x, y);
    let open = family(&mut ui).unwrap_or_else(|| panic!("a held press opens it"));
    assert_eq!(open["title"], "Move");
    let (corner, below) = (rect(&open["rect"]), rect(&face["rect"])[3]);
    assert!(
        (corner[0] - x).abs() <= 2.0 && (below..=below + 8.0).contains(&corner[1]),
        "at the pointer's x, {x}, just below the face, {below}: {corner:?}"
    );
    let brushes = ["Move", "Move Connected", "Move Elastic", "Pull Out"];
    let brushes = [&brushes[..], &["Pull Out Along Normal"]].concat();
    assert_eq!(
        ui.labels("family"),
        brushes,
        "the family, in brush.list's order"
    );
    assert_eq!(ran(&mut ui), before, "opening runs nothing");

    ui.press("family > Move Elastic");
    let after = ran(&mut ui);
    assert_eq!(after.len(), before.len() + 1, "one step: {after:?}");
    assert_eq!(after[..before.len()], before[..]);
    let pick = &after[before.len()];
    assert_eq!(
        (&pick["commands"], &pick["role"]),
        (&json!(["brush.set"]), &json!("human"))
    );
    assert_eq!(ui.state()["brush"], "Move Elastic");
    assert!(family(&mut ui).is_none(), "a pick shuts it");
    let face = ui.face("Move");
    assert_eq!(
        shown(&face),
        (json!("Move Elastic"), json!(true)),
        "now its face"
    );
    let lit = (ui.widgets("shelf").iter())
        .filter(|w| shown(w) == (json!("Move Elastic"), json!(true)))
        .count();
    assert_eq!(lit, 2, "the face and the favorite alike");
}

/// A tap on a lit face opens its family below the face, which it leaves bare, so a tap on the
/// same point shuts it; neither runs anything. Smooth's face, the slot's brush, is always lit.
#[test]
fn a_second_tap_on_the_same_point_of_a_lit_face_shuts_its_family_and_runs_nothing() {
    let mut ui = Ui::new();
    let face = ui.face("Smooth");
    assert_eq!(shown(&face), (json!("Smooth"), json!(true)));
    let before = ran(&mut ui);
    let [x, y] = center(&face["rect"]);
    ui.tap_at(x, y);
    ui.settle();
    assert!(family(&mut ui).is_some(), "a tap on a lit face opens it");
    ui.tap_at(x, y);
    ui.settle();
    assert!(
        family(&mut ui).is_none(),
        "a tap on the same point shuts it"
    );
    assert_eq!(ran(&mut ui), before, "neither runs anything");
}

/// Decision 0025, rule 8: every family's button says on hover which family it is and what
/// opening it does.
#[test]
fn every_family_button_has_a_hint() {
    let mut ui = Ui::new();
    for family in ["Build", "Carve", "Move", "Smooth", "Mask", "Paint"] {
        let face = ui.face(family);
        let hint = face["hint"].as_str().unwrap_or_default();
        assert!(hint.contains(family), "{family}: {face}");
    }
}

#[test]
fn escape_or_a_tap_outside_shuts_an_open_family_and_runs_nothing() {
    let mut ui = Ui::new();
    let before = ran(&mut ui);
    ui.open_family("Carve");
    assert!(family(&mut ui).is_some(), "a held press opens it");
    ui.tap("Escape", Modifiers::default());
    assert!(family(&mut ui).is_none(), "Escape shuts it");

    ui.open_family("Carve");
    assert!(family(&mut ui).is_some());
    // A tap on the Build family's label: outside the open family, on nothing that runs.
    ui.press("shelf > Build");
    assert!(family(&mut ui).is_none(), "a tap outside shuts it");
    assert_eq!(ran(&mut ui), before, "nothing ran");
}
