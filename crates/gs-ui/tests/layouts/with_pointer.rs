//! Keys that work by where the pointer is (decision 0025, section 5): F6 opens the Library's
//! Materials tab with the pointer off the 3D view, and over the view runs nothing.

use gs_platform::Modifiers;
use serde_json::json;

use crate::region::middle;
use crate::rig::Ui;

/// Catalog `material.palette`: "the big thumbnail opens the grid, also by key with the pointer
/// off the canvas", and `interface.default-bindings`' row "F6 materials, only with the pointer
/// outside the viewport".
#[test]
fn f6_opens_the_materials_tab_only_with_the_pointer_off_the_3d_view() {
    let mut ui = Ui::new();
    ui.settle();
    let [x, y] = ui.middle();
    ui.pen(x, y);
    ui.tap("F6", Modifiers::default());
    let shut = ui.ok("ui.library", json!({}));
    assert_eq!(
        shut["open"],
        json!(false),
        "over the 3D view F6 runs nothing"
    );

    let [x, y] = middle(ui.rect("status"));
    ui.pen(x, y);
    ui.tap("F6", Modifiers::default());
    let open = ui.ok("ui.library", json!({}));
    assert_eq!(
        (&open["open"], &open["tab"]),
        (&json!(true), &json!("matcaps")),
        "off the view, over the status line, F6 opens the Materials tab"
    );
}
