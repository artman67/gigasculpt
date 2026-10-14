//! What floats over the 3D view fits a short window (decision 0025, section 2): the header's
//! view buttons' options and the quick menu stay inside their room, and what does not fit scrolls,
//! dragged with the pen as a tray is, so the last of their controls is in reach. The settings
//! window's own check is in settings.rs.

use gs_platform::Modifiers;
use serde_json::json;

use crate::region::middle;
use crate::rig::{Ui, held};

/// The window's height in a short window, in pixels.
const SHORT: u32 = 600;

/// A short window showing the Workbench.
fn short() -> Ui {
    fit(Ui::new())
}

/// `ui` made a short window showing the Workbench.
fn fit(mut ui: Ui) -> Ui {
    ui.host.lock().resize([1800, SHORT], 1.0);
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    ui.settle();
    ui
}

/// The floating panel `id` lies inside `room` (pixels), and its last control, `last`, is in
/// reach: showing, or brought into sight by dragging the panel up with the pen, which leaves
/// the panel where it was.
fn in_reach(ui: &mut Ui, id: &str, room: [f64; 4], last: &str) {
    let panel = rests(ui, id);
    assert!(
        panel[1] >= room[1] && panel[3] <= room[3],
        "{id} {panel:?} inside {room:?}"
    );
    ui.scroll_to(id, last);
    let shown = ui.labels(id);
    assert_eq!(
        shown.last().map(String::as_str),
        Some(last),
        "{id} shows {shown:?}"
    );
    let [_, mid] = middle(ui.rect(&format!("{id} > {last}")));
    assert!(mid > panel[1] && mid < panel[3], "{last} in {panel:?}");
    assert_eq!(rests(ui, id), panel, "{id} stays where it was");
}

/// Where the floating panel `id` comes to rest, in pixels. A panel just opened or scrolled
/// settles over a frame or two (`ui.describe` draws one each time): egui sizes it from what it
/// held the frame before, and the header places a view button's options by that size.
fn rests(ui: &mut Ui, id: &str) -> [f64; 4] {
    let mut panel = ui.rect(id);
    for _ in 0..4 {
        let next = ui.rect(id);
        if next.iter().zip(&panel).all(|(a, b)| (a - b).abs() < 1e-6) {
            break;
        }
        panel = next;
    }
    panel
}

#[test]
fn the_look_options_fit_a_short_window() {
    // With the looks read, as the window reads them at start, the Material section ends the
    // options, on a MatCap look's last slider.
    let mut ui = fit(Ui::with_looks());
    ui.open_look();
    let view = ui.rect("viewport");
    in_reach(&mut ui, "look", view, "Scatter radius");
}

#[test]
fn the_frame_and_isolate_options_fit_a_short_window() {
    let mut ui = short();
    for _ in 0..30 {
        ui.ok("primitive.sphere", json!({}));
    }
    let view = ui.rect("viewport");
    ui.hold("header > Frame");
    in_reach(&mut ui, "frame", view, "Last edit");
    // A press outside the open options only shuts them (decision 0060 as amended for H5).
    ui.tap("Escape", Modifiers::default());
    // A part's button each: thirty are more than the view has room for. End Isolate shows
    // only while a part is alone, so the settings' row ends them.
    ui.hold("header > Isolate");
    in_reach(&mut ui, "isolate", view, "Click on canvas");
}

#[test]
fn the_quick_menu_fits_a_short_window() {
    let mut ui = short();
    // As the interface grows, the same window holds fewer points. The menu stays inside the 3D
    // view (decision 0060 as amended for UI-tidy).
    for scale in [1.0, 1.5] {
        ui.ok("ui.layout", json!({ "scale": scale }));
        let [x, y] = ui.middle();
        ui.pen(x, y);
        ui.tap("Space", held("Shift"));
        let view = ui.rect("viewport");
        in_reach(&mut ui, "quick", view, "Adjust");
        ui.tap("Escape", Modifiers::default());
    }
}

/// With two quick menus their buttons head the menu, and its room counts them: in the shortest
/// view above (the interface at 150%), the first menu stays inside the view, its pieces
/// scrolling under the buttons.
#[test]
fn two_quick_menus_fit_a_short_window() {
    let mut ui = short();
    ui.ok("ui.layout", json!({ "scale": 1.5 }));
    ui.ok("ui.quick", json!({ "menu": 2, "add": "history" }));
    ui.ok("ui.quick", json!({ "menu": 1 }));
    let [x, y] = ui.middle();
    ui.pen(x, y);
    ui.tap("Space", held("Shift"));
    let view = ui.rect("viewport");
    let panel = rests(&mut ui, "quick");
    let inside = panel[1] >= view[1] && panel[3] <= view[3];
    assert!(inside, "quick {panel:?} inside {view:?}");
    let labels = ui.labels("quick");
    let has = |label: &str| labels.iter().any(|l| l == label);
    assert!(has("Menu 2") && has("Size") && !has("Adjust"), "{labels:?}");
}
