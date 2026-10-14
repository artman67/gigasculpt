//! The Library (decision 0025, section 13): Library in the shelf's Quick group opens a drawer
//! along the bottom of the 3D view, which gets that much shorter; its tabs show their piece's
//! controls as tiles that run the same commands and light the same way; Close shuts it. A
//! tile's Pin makes it a favorite, on the shelf as the same control and kept in `layout.toml`.

use gs_protocol::ErrorCode;
use gs_ui::LayoutFile;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, folder};

/// Whether panel `id`'s widget `label` is drawn lit.
pub(crate) fn lit(ui: &mut Ui, id: &str, label: &str) -> bool {
    let widgets = ui.widgets(id);
    let widget = widgets.iter().find(|w| w["label"] == label);
    widget.is_some_and(|w| w["selected"] == true)
}

#[test]
fn the_library_drawer_opens_below_the_view_and_its_tiles_run_their_controls() {
    let mut ui = Ui::new();
    ui.settle();
    assert!(ui.widgets("library").is_empty(), "it starts shut");
    let view = ui.rect("viewport");
    ui.press("shelf > Library");
    assert_eq!(ui.state()["layout.library"], json!(true));
    assert!(lit(&mut ui, "shelf", "Library"), "lit while open");

    // A region along the 3D view's foot: the view ends above it.
    let drawer = ui.rect("library");
    let shorter = ui.rect("viewport");
    assert!(
        shorter[3] <= drawer[1] + 1.0,
        "{shorter:?} above {drawer:?}"
    );
    assert!(
        shorter[3] < view[3] - 50.0,
        "{shorter:?} shorter than {view:?}"
    );
    let labels = ui.labels("library");
    for tab in [
        "Brushes",
        "Alphas",
        "Materials",
        "Environments",
        "Layouts",
        "Close",
    ] {
        assert!(labels.iter().any(|l| l == tab), "{tab} in {labels:?}");
    }

    // A tile is its control: Ring sets the brush's alpha and is lit while it is the alpha.
    ui.press("library > Alphas");
    assert!(lit(&mut ui, "library", "Alphas"), "the tab shown is lit");
    assert!(!lit(&mut ui, "library", "Ring"));
    ui.press("library > Ring");
    assert_eq!(ui.state()["brush.alpha"], json!("ring"));
    assert!(
        lit(&mut ui, "library", "Ring"),
        "lit as its control would be"
    );
    let info = ui.ok("ui.library", json!({}));
    assert_eq!(info["tab"], json!("alphas"));

    let (code, message) = ui.err("ui.library", json!({ "tab": "hats" }));
    assert_eq!(code, ErrorCode::NotFound);
    assert!(message.contains("brushes, alphas"), "{message}");
    ui.press("library > Close");
    assert!(ui.widgets("library").is_empty(), "Close shuts it");
    assert_eq!(ui.state()["layout.library"], json!(false));
    assert_eq!(ui.rect("viewport"), view, "the view is whole again");
}

/// The Pin under the Alphas tile setting the brush's alpha to `alpha`.
fn pin_of(ui: &mut Ui, alpha: &str) -> Value {
    let widgets = ui.widgets("library");
    let pins = |w: &&Value| {
        let args = &w["command"]["args"];
        w["label"] == "Pin" && (args["pin"].is_object() || args["unpin"].is_object())
    };
    let of = |w: &&Value| {
        let args = &w["command"]["args"];
        args["pin"]["args"]["alpha"] == alpha || args["unpin"]["args"]["alpha"] == alpha
    };
    let pin = widgets.iter().filter(pins).find(of).cloned();
    pin.unwrap_or_else(|| panic!("no Pin under {alpha}"))
}

/// Taps the middle of `widget` as `ui.describe` reported it.
fn tap(ui: &mut Ui, widget: &Value) {
    let rect: [f64; 4] = std::array::from_fn(|i| widget["rect"][i].as_f64().unwrap_or_default());
    let [x, y] = middle(rect);
    ui.tap_at(x, y);
    ui.settle();
}

/// The shelf's widget labelled `label`.
fn on_shelf(ui: &mut Ui, label: &str) -> Option<Value> {
    ui.widgets("shelf")
        .into_iter()
        .find(|w| w["label"] == label)
}

#[test]
fn a_pinned_tile_is_its_own_control_on_the_shelf_and_stays_pinned_after_a_restart() {
    let path = folder("favorites").join("layout.toml");
    let load = || LayoutFile::load(&path).unwrap_or_else(|e| panic!("{e}"));
    let mut ui = Ui::with_layout(load());
    ui.settle();
    assert!(
        on_shelf(&mut ui, "Favorites").is_none(),
        "no favorites, no title"
    );
    ui.ok("ui.library", json!({ "tab": "alphas" }));
    ui.settle();
    let pin = pin_of(&mut ui, "ring");
    assert_eq!(pin["selected"], json!(false));
    tap(&mut ui, &pin);
    assert_eq!(
        pin_of(&mut ui, "ring")["selected"],
        json!(true),
        "lit while pinned"
    );

    // On the shelf, under Favorites, Ring is the tile's control: it runs and lights the same.
    assert!(on_shelf(&mut ui, "Favorites").is_some());
    let ring = on_shelf(&mut ui, "Ring").unwrap_or_else(|| panic!("Ring on the shelf"));
    let runs = json!({ "command": "brush.set", "args": { "alpha": "ring" } });
    assert_eq!(ring["command"], runs);
    tap(&mut ui, &ring);
    assert_eq!(ui.state()["brush.alpha"], json!("ring"));
    assert_eq!(
        on_shelf(&mut ui, "Ring").map(|w| w["selected"].clone()),
        Some(json!(true))
    );

    // An agent pins as a hand does, `at` a place; only what a tile runs, and unpins only a
    // favorite.
    let square = json!({ "command": "brush.set", "args": { "alpha": "square" } });
    let info = ui.ok("ui.library", json!({ "pin": square, "at": 0 }));
    assert_eq!(info["favorites"], json!([square, runs]));
    let hat = json!({ "command": "brush.set", "args": { "alpha": "hat" } });
    let (code, message) = ui.err("ui.library", json!({ "pin": hat }));
    assert_eq!(
        (code, message.contains("no Library tile")),
        (ErrorCode::NotFound, true)
    );
    let (code, _) = ui.err("ui.library", json!({ "unpin": hat }));
    assert_eq!(code, ErrorCode::NotFound);
    ui.settle();

    // Kept in layout.toml: a window started again has them, in order.
    let mut again = Ui::with_layout(load());
    again.settle();
    assert_eq!(
        again.ok("ui.library", json!({}))["favorites"],
        json!([square, runs])
    );
}

#[test]
fn a_favorite_no_tab_lists_is_drawn_off_and_a_tap_unpins_it() {
    let path = folder("favorites-gone").join("layout.toml");
    let text = "favorites = [\n  { command = \"brush.set\", args = { alpha = \"gone\" } },\n  \
                { command = \"view.frame\" },\n]\n";
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{e}"));
    let mut ui = Ui::with_layout(LayoutFile::load(&path).unwrap_or_else(|e| panic!("{e}")));
    ui.settle();
    let gone = on_shelf(&mut ui, "gone").unwrap_or_else(|| panic!("gone on the shelf"));
    assert_eq!(gone["command"]["command"], json!("ui.library"));
    assert_eq!(
        gone["command"]["args"]["unpin"]["args"]["alpha"],
        json!("gone")
    );
    tap(&mut ui, &gone);
    assert!(on_shelf(&mut ui, "gone").is_none(), "unpinned");
    // Written without arguments, a favorite has none.
    let left = json!([{ "command": "view.frame", "args": {} }]);
    assert_eq!(ui.ok("ui.library", json!({}))["favorites"], left);
}
