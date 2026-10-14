//! The settings window (decision 0025, sections 3 and 10): Header > Window opens it, it
//! holds the pen, navigation and the interface scale, and its Close button shuts it. The scale
//! multiplies the display's own content scale, so by default the interface follows the display,
//! and the user's scale survives a restart in `preferences.toml` (decision 0052 as amended).

use std::path::PathBuf;

use gs_platform::{Input, Modifiers};
use serde_json::json;

use crate::preferences::started;
use crate::region::middle;
use crate::rig::{Ui, center};

/// The status line's height, in pixels.
fn status_height(ui: &mut Ui) -> f64 {
    let [_, top, _, bottom] = ui.rect("status");
    bottom - top
}

#[test]
fn the_settings_window_sets_the_interface_scale_and_it_survives_a_restart() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("ui-settings-{}", std::process::id()));
    let mut ui = started(&dir);
    assert!(ui.widgets("settings").is_empty(), "it starts shut");
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    let labels = ui.labels("settings");
    for label in ["Pen", "Click", "Navigation", "Size sensitivity"] {
        assert!(labels.iter().any(|l| l == label), "{label} in {labels:?}");
    }
    let normal = status_height(&mut ui);
    let near = |a: f64, b: f64| (a - b).abs() <= 2.0;

    // The display asks for 125% beyond its pixels (an X11 desktop at 120 dpi): the interface
    // follows it while its own scale stays 100%.
    ui.host.lock().content_scale(1.25);
    ui.settle();
    let followed = status_height(&mut ui);
    assert!(near(followed, normal * 1.25), "{followed} vs {normal}");
    assert_eq!(ui.state()["layout.scale"], json!(1.0));
    // The scale is on the Interface page (decision 0052), which scrolls if it does not fit.
    ui.press("settings > Interface");
    ui.scroll_to("settings", "150%");
    assert!(ui.labels("settings").iter().any(|l| l == "Interface scale"));
    ui.press("settings > 150%");
    assert_eq!(ui.state()["layout.scale"], json!(1.5));
    let both = status_height(&mut ui);
    assert!(near(both, normal * 1.25 * 1.5), "{both} vs {normal}");
    ui.press("settings > Close");
    assert!(ui.widgets("settings").is_empty(), "Close shuts it");

    // A new window on a display without a content scale: 150% of normal.
    drop(ui);
    let text = std::fs::read_to_string(dir.join("preferences.toml")).unwrap_or_default();
    assert!(text.contains("scale = 1.5"), "saved: {text}");
    let mut again = started(&dir);
    again.settle();
    assert_eq!(again.state()["layout.scale"], json!(1.5));
    let restarted = status_height(&mut again);
    assert!(near(restarted, normal * 1.5), "{restarted} vs {normal}");
}

/// Decision 0060, section 6: the gizmo's size on screen is a setting in Settings > Cursor, out of
/// the Gizmo tool's tray. The wheel over it steps it a hundredth of its 10 to 200 rail.
#[test]
fn the_gizmo_size_is_set_in_settings_cursor() {
    let mut ui = Ui::new();
    ui.ok("ui.settings", json!({ "open": true, "page": "Cursor" }));
    ui.settle();
    let size = |ui: &mut Ui| ui.state()["gizmo.size"].as_f64().unwrap_or_default();
    let before = size(&mut ui);
    let [x, y] = middle(ui.rect("settings > Gizmo size"));
    ui.pen(x, y);
    ui.input(&[Input::Wheel { dx: 0.0, dy: 1.0 }], Modifiers::default());
    let after = size(&mut ui);
    assert!((after - before - 1.9).abs() < 1e-3, "{before} to {after}");
}

/// Decision 0060, sections 8 to 10: the settings window's sets in compact rows. A choice of a
/// few values is one row after its short label, each button by its short label (section 9's
/// pen and Navigation rows, and the rest by the same rule); a long label is a short one with the
/// sentence in its hint ("Empty press turns view", "Name held keys").
#[test]
fn a_choice_in_settings_is_one_row_of_short_labels() {
    let mut ui = Ui::new();
    let rows: [(&str, &str, &[&str]); 10] = [
        (
            "Pen",
            "Eraser",
            &["Tip", "Shift", "Ctrl", "Alt", "Ctrl+Alt"],
        ),
        ("Pen", "Click", &["Wheel", "Palette", "Quick", "None"]),
        ("Pen", "Button 2", &["Pan", "Shift", "Ctrl", "Alt"]),
        ("Interface", "Sticky", &["Modifiers", "Slider keys"]),
        ("Navigation", "Orbit", &["Free", "X", "Y", "Z"]),
        (
            "Navigation",
            "Pivot",
            &["Middle", "Last edit", "Parts", "Pointer"],
        ),
        ("Navigation", "Snap", &["5°", "15°", "30°", "45°", "90°"]),
        ("Cursor", "Rings", &["On surface", "Mirrored"]),
        ("Files", "Brush at start", &["Last used", "Pinned"]),
        ("Geometry", "Hole closing", &["None", "Large triangles"]),
    ];
    for (page, line, buttons) in rows {
        ui.ok("ui.settings", json!({ "open": true, "page": page }));
        ui.settle();
        ui.scroll_to("settings", line);
        let widgets = ui.widgets("settings");
        let at = widgets.iter().position(|w| w["label"] == line);
        let at = at.unwrap_or_else(|| panic!("{page} > {line}: {widgets:?}"));
        let row = &widgets[at..=at + buttons.len()];
        let labels: Vec<&str> = row[1..]
            .iter()
            .filter_map(|w| w["label"].as_str())
            .collect();
        assert_eq!(labels, buttons, "{page} > {line}");
        let top = center(&row[0]["rect"])[1];
        let off = row
            .iter()
            .filter(|w| (center(&w["rect"])[1] - top).abs() > 6.0);
        assert_eq!(off.count(), 0, "{page} > {line} is one row: {row:?}");
    }
    let labels = |ui: &mut Ui, page: &str| {
        ui.ok("ui.settings", json!({ "open": true, "page": page }));
        ui.settle();
        ui.labels("settings")
    };
    assert!(labels(&mut ui, "Navigation").contains(&"Empty press turns view".into()));
    assert!(labels(&mut ui, "Interface").contains(&"Name held keys".into()));
}

/// Decision 0060, section 7: Keys is Settings' Keys page, not a scene tray section. The page
/// starts with Bind a key… and lists every key under its context's heading.
#[test]
fn the_keys_are_a_page_of_settings_not_a_section_of_the_scene_tray() {
    let mut ui = Ui::new();
    ui.host.lock().resize([1800, 6000], 1.0);
    ui.host.lock().shell.layout.layout.folded.clear();
    ui.settle();
    let tray = ui.labels("tray");
    assert!(!tray.iter().any(|l| l.ends_with(" keys")), "{tray:?}");
    let pages = ui.ok("ui.settings", json!({}))["pages"].clone();
    assert_eq!(
        pages.as_array().and_then(|p| p.last()),
        Some(&json!("Keys"))
    );
    ui.ok("ui.settings", json!({ "open": true, "page": "Keys" }));
    ui.settle();
    let labels = ui.labels("settings");
    for label in [
        "Bind a key…",
        "General keys",
        "Geometry keys",
        "Ctrl+Z: Undo",
    ] {
        assert!(labels.iter().any(|l| l == label), "{label} in {labels:?}");
    }
}

/// The settings window draws where it settles from its first frame: at Keys, a page taller than
/// its room, with the header on two rows (as at the test window's size), the frame after the
/// first moves nothing.
#[test]
fn the_settings_window_draws_where_it_settles_from_its_first_frame() {
    let mut ui = Ui::new();
    ui.ok("ui.settings", json!({ "open": true, "page": "Keys" }));
    ui.settle();
    // As the last frame drew it, without drawing another.
    let bind = |ui: &mut Ui| {
        let studio = ui.host.lock();
        let settings = studio.shell.panels().iter().find(|p| p.id == "settings");
        let widgets = settings.map(|p| p.widgets.as_slice()).unwrap_or_default();
        widgets
            .iter()
            .find(|w| w.label == "Bind a key…")
            .map(|w| w.rect)
    };
    let first = bind(&mut ui);
    assert!(first.is_some(), "Bind a key… shows");
    ui.settle();
    assert_eq!(bind(&mut ui), first, "the second frame moved it");
}

/// In a window 600 points high the settings window's pieces scroll, so it stays inside the 3D
/// view and Close, at its foot, shows and shuts it.
#[test]
fn the_settings_close_button_is_reachable_in_a_short_window() {
    let mut ui = Ui::new();
    ui.host.lock().resize([1800, 600], 1.0);
    ui.ok("ui.settings", json!({ "open": true }));
    ui.settle();
    let view = ui.rect("viewport");
    let window = ui.rect("settings");
    assert!(
        window[3] <= view[3],
        "{window:?} inside the 3D view {view:?}"
    );
    let close = ui.rect("settings > Close");
    assert!(
        close[3] <= window[3],
        "Close shows: {close:?} in {window:?}"
    );
    ui.press("settings > Close");
    assert!(ui.widgets("settings").is_empty(), "Close shuts it");
}

/// Decision 0060 as amended for H5, item 4: with no view strip, the axis figure and the
/// settings window sit under the header in the 3D view's pen-side top corner, by the scene
/// tray: the view reaches the tray (its margin and divider apart), the figure's centre 68 points
/// in from the view's edge, the window a margin from it.
#[test]
fn the_figure_and_the_settings_window_sit_under_the_header_by_the_scene_tray() {
    let mut ui = Ui::new();
    let (header, tray, figure) = (ui.rect("header"), ui.rect("tray"), ui.rect("figure"));
    let view = ui.rect("viewport");
    assert!(
        (0.0..=10.0).contains(&(tray[0] - view[2])),
        "the view {view:?} reaches the scene tray {tray:?}"
    );
    assert!(
        figure[1] >= header[3] - 0.5,
        "the figure {figure:?} under the header {header:?}"
    );
    let centre = f64::midpoint(figure[0], figure[2]);
    assert!(
        (view[2] - 68.0 - centre).abs() <= 2.0,
        "the figure {figure:?} 68 points in from the view's edge {view:?}"
    );
    ui.ok("ui.settings", json!({ "open": true }));
    ui.settle();
    let window = ui.rect("settings");
    assert!(
        window[1] >= header[3] - 0.5,
        "the settings {window:?} under the header {header:?}"
    );
    assert!(
        (0.0..=20.0).contains(&(tray[0] - window[2])),
        "the settings {window:?} by the scene tray {tray:?}"
    );
}

/// Review 0009, R14: Escape closes the settings window, as Close does. Typing in its search or
/// in a slider's value, the first Escape only lets go of the field; with Reset's question up,
/// Escape answers Cancel and the window stays.
#[test]
fn escape_closes_the_settings_window_once_no_field_is_typed_in() {
    let none = Modifiers::default();
    let mut ui = Ui::new();
    ui.ok("ui.navigation", json!({ "barrel_click": "palette" }));
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    ui.press("settings > Search");
    ui.input(&[Input::Text("pivot".into())], none);
    ui.tap("Escape", none);
    assert!(
        !ui.widgets("settings").is_empty(),
        "the search let go first"
    );
    ui.press("settings > Navigation");
    let speed = ui.rect("settings > Orbit speed");
    ui.tap_at(speed[2] - 4.0, middle(speed)[1]);
    ui.input(&[Input::Text("3".into())], none);
    ui.tap("Escape", none);
    assert!(
        !ui.widgets("settings").is_empty(),
        "the slider's value let go first"
    );
    ui.press("settings > Reset all");
    assert!(!ui.widgets("reset").is_empty(), "Reset asks first");
    ui.tap("Escape", none);
    assert!(ui.widgets("reset").is_empty(), "Escape answers Cancel");
    assert!(!ui.widgets("settings").is_empty(), "and the window stays");
    ui.tap("Escape", none);
    assert!(ui.widgets("settings").is_empty(), "Escape closed it");
    let barrel = ui.state()["navigation.barrel_click"].clone();
    assert_eq!(barrel, json!("palette"), "and reset nothing");
}
