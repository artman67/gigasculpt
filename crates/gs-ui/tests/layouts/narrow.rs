//! An 800-wide window (decision 0016, section 17, item 10; Narrow-win): the status line stays
//! inside its bar, beside the Pause agents toggle, and the shelf's scroll bar sits at the
//! shelf's edge, as the trays' do. The header's view buttons take a row of their own under the
//! menus before the menus wrap (decision 0060 as amended for H5, item 5).

use serde_json::{Value, json};

use crate::header::{VIEW_BUTTONS, left_to_right, titles_and_ends, with_header};
use crate::rig::{Ui, listed};
use crate::shelf::rect;

/// Panel `id`'s rectangle in `d`, in pixels.
fn panel(d: &Value, id: &str) -> [f64; 4] {
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let found = panels.into_iter().find(|p| p["id"] == id);
    let found = found.unwrap_or_else(|| panic!("no panel {id}: {d}"));
    rect(&found["rect"])
}

/// Catalog `interface.scene-files` and decision 0025, rule 12 (a row never widens its panel):
/// at 800 by 600 points with a sphere in the scene, every label of the status line ends left of
/// the Pause agents toggle, inside the bar; at 800 by 400, where the shelf scrolls, its bar
/// reaches the shelf's edge.
#[test]
fn at_800_wide_the_status_line_stays_in_its_bar_and_the_shelf_bar_reaches_its_edge() {
    let mut ui = Ui::new();
    ui.host.lock().resize([800, 600], 1.0);
    ui.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    ui.settle();
    let d = ui.describe();
    let bar = panel(&d, "status");
    let widgets = listed(&d, "status", "widgets");
    let toggle = widgets.iter().find(|w| w["label"] == "Pause agents");
    let toggle = rect(&toggle.unwrap_or_else(|| panic!("the toggle: {widgets:?}"))["rect"]);
    assert!(
        toggle[2] <= bar[2] + 0.5,
        "the toggle {toggle:?} in {bar:?}"
    );
    let labels: Vec<&Value> = widgets.iter().filter(|w| w["role"] == "label").collect();
    assert!(!labels.is_empty(), "the line shows something: {widgets:?}");
    for w in labels {
        let at = rect(&w["rect"]);
        assert!(
            at[0] >= bar[0] - 0.5 && at[2] <= toggle[0] + 0.5,
            "{} at {at:?} runs past the room beside the toggle {toggle:?}",
            w["label"]
        );
    }
    ui.host.lock().resize([800, 400], 1.0);
    ui.settle();
    ui.settle();
    let d = ui.describe();
    let shelf = panel(&d, "shelf");
    let widgets = listed(&d, "shelf", "widgets");
    let scroll = widgets.iter().find(|w| w["role"] == "scroll bar");
    let scroll = scroll.unwrap_or_else(|| panic!("the shelf scrolls at 400 high: {widgets:?}"));
    let scroll = rect(&scroll["rect"]);
    assert!(
        (scroll[2] - shelf[2]).abs() <= 0.5,
        "the shelf's bar {scroll:?} sits at its edge {shelf:?}"
    );
}

/// Decision 0060 as amended for H5, item 5: with three menus the header is one row at full
/// width; at 800 wide the view buttons, all of them in their order, move to a row under the
/// menus, which stay on one row, and the bar holds them all with no scroll bar.
#[test]
fn narrow_the_view_buttons_take_a_second_row_before_the_menus_wrap() {
    let mut ui = Ui::new();
    with_header(&mut ui, &["file", "tone", "lights"], &["view", "look"]);
    let (titles, ends) = titles_and_ends(&mut ui);
    let tops: Vec<f64> = titles.iter().chain(&ends).map(|(_, r)| r[1]).collect();
    let one_row = |tops: &[f64]| tops.iter().all(|t| (t - tops[0]).abs() <= 1.0);
    assert!(one_row(&tops), "one row at full width: {tops:?}");
    ui.host.lock().resize([800, 600], 1.0);
    ui.settle();
    ui.settle();
    let (titles, ends) = titles_and_ends(&mut ui);
    assert_eq!(
        left_to_right(&ends),
        VIEW_BUTTONS[..7],
        "every view button shows"
    );
    let tops: Vec<f64> = titles.iter().map(|(_, r)| r[1]).collect();
    assert!(one_row(&tops), "the menus stay on one row: {titles:?}");
    let menus_bottom = titles.iter().map(|(_, r)| r[3]).fold(f64::MIN, f64::max);
    for (label, at) in &ends {
        assert!(
            at[1] >= menus_bottom - 0.5,
            "{label} {at:?} moved under the menus"
        );
    }
    let d = ui.describe();
    let bar = panel(&d, "header");
    let widgets = listed(&d, "header", "widgets");
    assert!(
        !widgets.iter().any(|w| w["role"] == "scroll bar"),
        "it never scrolls"
    );
    for w in &widgets {
        let at = rect(&w["rect"]);
        assert!(
            at[1] >= bar[1] - 0.5 && at[3] <= bar[3] + 0.5,
            "{w} in {bar:?}"
        );
    }
}
