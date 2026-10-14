//! A panel that scrolls shows its bar beside its controls, never over them (decision 0025,
//! rule 11): a tap at the end of a slider's value in a tray that scrolls opens the value's
//! field, and the bar `ui.describe` reports touches no control or section of either tray.

use gs_platform::Modifiers;
use serde_json::Value;

use crate::region::middle;
use crate::rig::{Ui, listed};

/// A rectangle `ui.describe` reports, in pixels.
fn pixels(rect: &Value) -> [f64; 4] {
    std::array::from_fn(|i| rect[i].as_f64().unwrap_or_default())
}

/// A panel's scroll bar, if it shows one, and everything else it draws, by name.
type Drawn = (Option<[f64; 4]>, Vec<(String, [f64; 4])>);

/// Panel `id`'s scroll bar and everything else it draws (its widgets and sections), by name.
fn drawn(ui: &mut Ui, id: &str) -> Drawn {
    let d = ui.describe();
    let (mut bar, mut rest) = (None, Vec::new());
    for w in listed(&d, id, "widgets") {
        match w["role"].as_str() {
            Some("scroll bar") => bar = Some(pixels(&w["rect"])),
            _ => rest.push((w["label"].to_string(), pixels(&w["rect"]))),
        }
    }
    for s in listed(&d, id, "sections") {
        rest.push((format!("section {}", s["id"]), pixels(&s["rect"])));
    }
    (bar, rest)
}

/// Whether two rectangles share any area (touching edges share none).
fn overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Checks that panel `id` scrolls and that its bar overlaps nothing else it draws.
fn clear_of_the_bar(ui: &mut Ui, id: &str, at: &str) {
    let (bar, rest) = drawn(ui, id);
    let bar = bar.unwrap_or_else(|| panic!("the {id} tray scrolls and reports its bar ({at})"));
    let covered: Vec<&String> = (rest.iter())
        .filter(|(_, r)| overlap(bar, *r))
        .map(|(name, _)| name)
        .collect();
    assert!(
        covered.is_empty(),
        "the {id} tray's bar {bar:?} lies over {covered:?} ({at})"
    );
}

/// With the brush tray scrolling (Depth open makes it longer than the window), a tap 4 points in
/// from the right end of a slider's value opens its field, at the top of the tray and scrolled
/// down to Alpha.
#[test]
fn a_tap_at_the_end_of_a_slider_value_in_a_scrolling_tray_opens_its_field() {
    let mut ui = Ui::new();
    let open = ["head", "brush", "stroke", "alpha", "paint", "depth"];
    ui.open_only_in("brush", &open);
    for name in ["Strength", "Spacing", "Follow"] {
        let r = ui.rect(&format!("brush > {name}"));
        ui.tap_at(r[2] - 4.0, middle(r)[1]);
        let field = format!("Type {name}");
        let widgets = ui.widgets("brush");
        let open = (widgets.iter()).any(|w| w["label"] == field && w["role"] == "text field");
        assert!(open, "a tap at the end of {name}'s value opens its field");
        ui.tap("Escape", Modifiers::default());
    }
    assert!(
        drawn(&mut ui, "brush").0.is_some(),
        "the brush tray scrolls"
    );
}

/// Every section of both trays open, each tray's bar overlaps none of its controls or
/// sections, at the top and scrolled to its last section. A window 400 points high, so the
/// scene tray scrolls without the keys sheet, which is Settings' Keys page since R2.
#[test]
fn a_trays_scroll_bar_lies_beside_its_controls_not_over_them() {
    let mut ui = Ui::new();
    ui.host.lock().resize([1800, 400], 1.0);
    ui.host.lock().shell.layout.layout.folded.clear();
    ui.settle();
    for (id, last) in [("brush", "alpha"), ("tray", "history")] {
        clear_of_the_bar(&mut ui, id, "at the top");
        ui.scroll_to(id, last);
        clear_of_the_bar(&mut ui, id, "scrolled");
    }
}
