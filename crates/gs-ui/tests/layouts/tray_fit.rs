//! No row runs past its panel's edge (decision 0025, rule 12): with every section of both trays
//! open, at the top and scrolled to the end, every widget and section the trays and the shelf
//! report lies inside its panel, left of its scroll bar while it scrolls, none cut short at the
//! side, and the trays and the 3D view lie where they do with every section shut;
//! and a wrapped line of the keys sheet ends above the next. A leveled part with Live Detail
//! on fills Parts' title and every Detail row; the narrow window has Live Detail on alone.

use serde_json::{Value, json};

use crate::rig::{Ui, listed};

/// The gap between a panel's controls and its scroll bar (rule 11), in pixels at the rig's
/// scale of 1.
const GAP: f64 = 4.0;

/// The panels whose places the trays' widths decide.
const PLACED: [&str; 3] = ["brush", "tray", "viewport"];

/// A rectangle `ui.describe` reports, in pixels.
fn pixels(rect: &Value) -> [f64; 4] {
    std::array::from_fn(|i| rect[i].as_f64().unwrap_or_default())
}

/// Where panel `id` lies in `ui.describe`'s answer `d`.
fn panel(d: &Value, id: &str) -> [f64; 4] {
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let found = panels.into_iter().find(|p| p["id"] == id);
    pixels(&found.unwrap_or_else(|| panic!("a {id} panel"))["rect"])
}

/// Whether `a` lies inside `b`.
fn inside(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] >= b[0] && a[1] >= b[1] && a[2] <= b[2] && a[3] <= b[3]
}

/// Folds every section of both trays shut, or opens every one, and draws the window again.
fn fold_all(ui: &mut Ui, shut: bool) {
    let mut studio = ui.host.lock();
    let shell = &mut studio.shell;
    let shown = shell.directions.get(shell.shown());
    let trays = shown.map(|d| {
        d.panels
            .iter()
            .filter(|p| p.id == "brush" || p.id == "tray")
    });
    let ids: Vec<String> =
        trays.map_or_else(Vec::new, |t| t.flat_map(|p| p.show.clone()).collect());
    shell.layout.layout.folded = if shut { ids } else { Vec::new() };
    drop(studio);
    ui.settle();
}

/// What panel `id` draws past its edge: anything outside the panel or right of its controls'
/// edge (a gap short of its scroll bar, while it scrolls), and any widget the panel's list cut
/// short at the side (one cut at the top or bottom touches the panel's top or bottom).
fn past(d: &Value, id: &str) -> Vec<String> {
    let p = panel(d, id);
    let widgets = listed(d, id, "widgets");
    let bar = (widgets.iter())
        .find(|w| w["role"] == "scroll bar")
        .map(|w| pixels(&w["rect"]));
    let mut out: Vec<String> = bar
        .filter(|b| !inside(*b, p) || (b[2] - p[2]).abs() > 0.5)
        .map(|b| format!("its bar {b:?}, not at the panel's edge {p:?}"))
        .into_iter()
        .collect();
    let edge = bar.map_or(p, |b| [p[0], p[1], b[0] - GAP, p[3]]);
    for w in widgets.iter().filter(|w| w["role"] != "scroll bar") {
        let r = pixels(&w["rect"]);
        let side = w["cut"] == true && r[1] > p[1] + 1.0 && r[3] < p[3] - 1.0;
        if !inside(r, edge) || side {
            out.push(format!("{} {r:?}, cut {}", w["label"], w["cut"]));
        }
    }
    for s in listed(d, id, "sections") {
        let r = pixels(&s["rect"]);
        if !inside(r, edge) {
            out.push(format!("section {} {r:?}", s["id"]));
        }
    }
    out
}

/// The scene tray's labels that run into the next one down: a wrapped line of the keys sheet
/// (`Ctrl+Shift+T: ui.palette args = ...`) over the line below it.
fn overrun(d: &Value) -> Vec<String> {
    let lines: Vec<(Value, [f64; 4])> = listed(d, "tray", "widgets")
        .into_iter()
        .filter(|w| w["role"] == "label")
        .map(|w| (w["label"].clone(), pixels(&w["rect"])))
        .collect();
    let meet = |a: [f64; 4], b: [f64; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
    (lines.windows(2))
        .filter(|w| meet(w[0].1, w[1].1))
        .map(|w| format!("{} {:?} over {} {:?}", w[0].0, w[0].1, w[1].0, w[1].1))
        .collect()
}

/// Checks that the trays and the shelf draw nothing past their edges, and that the panels lie
/// where they did with every section shut (`shut`).
fn fits(ui: &mut Ui, shut: &[[f64; 4]; 3], at: &str) {
    let d = ui.describe();
    for (id, was) in PLACED.iter().zip(shut) {
        assert_eq!(panel(&d, id), *was, "the {id} panel keeps its place ({at})");
    }
    for id in ["brush", "tray", "shelf"] {
        let past = past(&d, id);
        assert!(
            past.is_empty(),
            "the {id} panel draws past its edge ({at}): {past:#?}"
        );
    }
}

/// Every section of both trays open, at the top, then each tray scrolled to its last section:
/// nothing runs past a panel's edge and no tray widens.
#[test]
fn no_row_runs_past_its_panel_and_each_tray_keeps_its_width() {
    // A part with three detail levels, the middle one active, and Live Detail on: Parts' title
    // names it, and Detail shows its stepper and every Live row (decision 0060 as amended for S1).
    let mut ui = crate::detail::ball(3);
    ui.ok("level.step", json!({ "by": -1 }));
    ui.ok("live_detail.set", json!({ "on": true }));
    fold_all(&mut ui, true);
    let d = ui.describe();
    let shut = PLACED.map(|id| panel(&d, id));
    fold_all(&mut ui, false);
    fits(&mut ui, &shut, "every section open, at the top");
    ui.scroll_to("brush", "alpha");
    fits(&mut ui, &shut, "the brush tray at its end");
    ui.scroll_to("tray", "history");
    fits(&mut ui, &shut, "both trays at their end");
    let overrun = overrun(&ui.describe());
    assert!(overrun.is_empty(), "wrapped lines overlap: {overrun:#?}");
}

/// A window narrow enough to shrink both trays to about a hundred points (800 wide), every
/// section open: at the top and with the scene tray at its last section, nothing runs past
/// either tray, and the 3D view lies beside the scene tray, not under it.
#[test]
fn a_narrow_windows_trays_fit_and_the_view_lies_beside_them() {
    let mut ui = Ui::new();
    ui.ok("live_detail.set", json!({ "on": true }));
    ui.host.lock().resize([800, 1000], 1.0);
    fold_all(&mut ui, false);
    for at in ["at the top", "the scene tray at its end"] {
        if at != "at the top" {
            ui.scroll_to("tray", "history");
        }
        let d = ui.describe();
        for id in ["brush", "tray"] {
            let past = past(&d, id);
            assert!(
                past.is_empty(),
                "the {id} panel draws past its edge ({at}): {past:#?}"
            );
        }
        let (view, tray) = (panel(&d, "viewport"), panel(&d, "tray"));
        assert!(
            view[2] <= tray[0],
            "the view {view:?} under the tray {tray:?} ({at})"
        );
    }
}
