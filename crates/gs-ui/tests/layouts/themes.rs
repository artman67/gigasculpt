//! The theme picker (decision 0025, section 13): the settings window's Previous and Next (the
//! Previous theme and Next theme controls) step through the themes in order and wrap, and a
//! button per theme is lit for the one
//! shown; the Library's Themes tab lists the same. The theme applies on the next frame and is
//! kept in `preferences.toml` (decision 0052 as amended).

use gs_protocol::ErrorCode;
use serde_json::json;

use crate::preferences::started;
use crate::rig::{Ui, folder};

/// The theme the state says is shown, the theme the window draws with, and which of the
/// settings window's theme buttons are lit. They sit at its foot, which scrolls in the window
/// the tests draw (decision 0025, section 2).
fn shown(ui: &mut Ui) -> (String, String, Vec<String>) {
    ui.scroll_to("settings", "High contrast");
    let state = ui.state()["layout.theme"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let titles = ["Dark", "Light", "High contrast"];
    let widgets = ui.widgets("settings");
    let lit = (widgets.iter())
        .filter(|w| w["selected"] == true && titles.iter().any(|t| w["label"] == *t))
        .filter_map(|w| Some(w["label"].as_str()?.to_string()))
        .collect();
    let drawn = ui.host.lock().theme().name.clone();
    (state, drawn, lit)
}

#[test]
fn the_theme_picker_steps_through_the_themes_and_the_choice_survives_a_restart() {
    let dir = folder("themes");
    let mut ui = started(&dir);
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    ui.press("settings > Interface");
    let is = |name: &str, title: &str| (name.to_string(), name.to_string(), vec![title.into()]);
    assert_eq!(shown(&mut ui), is("dark", "Dark"));
    ui.press("settings > Next");
    assert_eq!(shown(&mut ui), is("light", "Light"));
    ui.press("settings > Next");
    assert_eq!(shown(&mut ui), is("high-contrast", "High contrast"));
    ui.press("settings > Next");
    assert_eq!(
        shown(&mut ui),
        is("dark", "Dark"),
        "Next wraps to the first"
    );
    ui.press("settings > Previous");
    assert_eq!(shown(&mut ui), is("high-contrast", "High contrast"));
    ui.press("settings > Light");
    assert_eq!(shown(&mut ui), is("light", "Light"));

    let (code, message) = ui.err("ui.layout", json!({ "theme": "sepia" }));
    assert_eq!(code, ErrorCode::NotFound);
    assert!(message.contains("dark, light, high-contrast"), "{message}");
    let (code, _) = ui.err("ui.layout", json!({ "theme": "dark", "theme_step": 1 }));
    assert_eq!(code, ErrorCode::ArgsInvalid);
    let (code, _) = ui.err("ui.layout", json!({ "theme_step": 2 }));
    assert_eq!(code, ErrorCode::ArgsInvalid);
    let info = ui.ok("ui.layout", json!({}));
    assert_eq!(info["themes"], json!(["dark", "light", "high-contrast"]));

    // The Library's Themes tab: a tile per theme, which shows it.
    ui.ok("ui.settings", json!({ "open": false }));
    ui.ok("ui.library", json!({ "tab": "themes" }));
    ui.settle();
    ui.press("library > High contrast");
    assert_eq!(ui.state()["layout.theme"], json!("high-contrast"));

    let mut again = started(&dir);
    assert_eq!(again.state()["layout.theme"], json!("high-contrast"));
}
