//! Icons sit beside labels (decision 0025, section 13), so a button with one is wider, or alone
//! on the header's view buttons (decision 0060 as amended for H5): every label on the header,
//! the shelf, the scene tray, the settings window and the header's File menu still fits on one
//! line, so no button there is taller than one with a single line.

use serde_json::{Value, json};

use crate::rig::{Ui, folder};

/// The buttons of panel `id`, each with its height in pixels.
fn heights(ui: &mut Ui, id: &str) -> Vec<(String, f64)> {
    let height =
        |w: &Value| w["rect"][3].as_f64().unwrap_or(0.0) - w["rect"][1].as_f64().unwrap_or(0.0);
    let widgets = ui.widgets(id).into_iter();
    let buttons =
        widgets.filter(|w| ["button", "toggle"].contains(&w["role"].as_str().unwrap_or("")));
    buttons
        .map(|w| {
            (
                w["label"].as_str().unwrap_or_default().to_string(),
                height(&w),
            )
        })
        .collect()
}

/// Every button of the header is one line tall: none is taller than Frame.
fn header_on_one_line(ui: &mut Ui) {
    let shown = heights(ui, "header");
    let one_line = (shown.iter().find(|(label, _)| label == "Frame"))
        .map_or_else(|| panic!("the header shows Frame"), |(_, h)| *h);
    for (label, height) in shown {
        assert!(
            height <= one_line + 0.5,
            "{label} is {height} pixels tall, over one line's {one_line}"
        );
    }
}

#[test]
fn every_label_beside_an_icon_stays_on_one_line() {
    let mut ui = Ui::new();
    ui.ok("ui.settings", json!({ "open": true }));
    ui.open_only(&["detail"]);
    ui.ok("ui.menu", json!({ "open": "file" }));
    let one_line = heights(&mut ui, "header")
        .into_iter()
        .find(|(label, _)| label == "Frame")
        .map_or_else(|| panic!("the header shows Frame"), |(_, h)| h);
    for panel in ["header", "shelf", "tray", "settings", "menu"] {
        let shown = heights(&mut ui, panel);
        assert!(!shown.is_empty(), "{panel} shows buttons");
        for (label, height) in shown {
            assert!(
                height <= one_line + 0.5,
                "{label} in {panel} is {height} pixels tall, over one line's {one_line}"
            );
        }
    }
}

/// The header's view buttons grow with the type (decision 0025, section 6): a user's theme with
/// type half as large again widens them, Frame to Look, and no label there wraps.
#[test]
fn the_view_buttons_grow_for_larger_type_and_keep_their_labels_on_one_line() {
    let dark = include_str!("../../../../assets/themes/dark.toml");
    let colors = dark.split("[type]").next().unwrap_or_default();
    let s = &gs_ui::Themes::default().list()[0].sizes;
    let (small, body, heading, mono) = (s.small, s.body, s.heading, s.mono);
    let big = format!(
        "{}[type]\nsmall = {}\nbody = {}\nheading = {}\nmono = {}\n",
        colors.replacen("\"Dark\"", "\"Big type\"", 1),
        small * 1.5,
        body * 1.5,
        heading * 1.5,
        mono * 1.5,
    );
    let dir = folder("big-type");
    std::fs::write(dir.join("big.toml"), big).unwrap_or_else(|e| panic!("{e}"));
    let (themes, problems) = gs_ui::Themes::load(&dir);
    assert_eq!(problems, Vec::<String>::new());
    let mut ui = Ui::new();
    ui.host.lock().shell.themes = themes;
    let width = |ui: &mut Ui| ui.rect("header > Look")[2] - ui.rect("header > Frame")[0];
    let narrow = width(&mut ui);
    ui.ok("ui.layout", json!({ "theme": "big" }));
    ui.settle();
    let wide = width(&mut ui);
    assert!(
        wide > narrow + 20.0,
        "the view buttons widen: {narrow} then {wide}"
    );
    header_on_one_line(&mut ui);
}
