//! An arrow, Enter or Tab goes to the one thing in front, as one Escape does (decision 0025,
//! section 5, amended 2026-10-14): the window gives each press to the first thing open in its
//! order, a text field with the keyboard first, and nothing behind that one sees the key.

use gs_platform::{Input, Modifiers};
use serde_json::json;

use crate::paint::type_hex;
use crate::recovery::{OFFER, buttons, crash, launched};
use crate::rig::{Ui, folder};

/// The palette's highlighted row.
fn row(ui: &Ui) -> usize {
    ui.host.lock().shell.palette.selected
}

#[test]
fn an_arrow_with_the_recovery_question_over_the_palette_moves_no_row_behind_it() {
    let none = Modifiers::default();
    let dir = folder("front-arrow");
    crash(&dir);
    let mut ui = launched(&dir);
    ui.tap("Space", none);
    // A few rows, so the palette along the top leaves the dialog's buttons clear.
    ui.input(&[Input::Text("sphere".into())], none);
    assert!(
        ui.labels("palette").len() > 2,
        "rows to move through: {:?}",
        ui.labels("palette")
    );
    ui.press("recovery > Discard");
    let asked = buttons(&mut ui);
    assert_eq!(asked, ["Delete", "Cancel"], "the question is asked, on top");
    let before = row(&ui);
    ui.tap("Down", none);
    assert_eq!(row(&ui), before, "the row behind the question stays");
    ui.press("recovery > Cancel");
    ui.press("recovery > Later");
    assert!(buttons(&mut ui).is_empty(), "the dialog is put away");
    ui.tap("Down", none);
    assert_eq!(row(&ui), before + 1, "with the palette in front it moves");
}

#[test]
fn tab_with_settings_open_leaves_the_panels_behind_it() {
    let none = Modifiers::default();
    let mut ui = Ui::new();
    let chrome = |ui: &mut Ui| ui.state()["layout.chrome"].clone();
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    assert!(!ui.widgets("settings").is_empty(), "Settings opens");
    ui.tap("Tab", none);
    assert_eq!(chrome(&mut ui), json!(true), "the panels behind it stay");
    assert!(!ui.widgets("settings").is_empty(), "and Settings stays");
    ui.tap("Escape", none);
    ui.tap("Tab", none);
    assert_eq!(chrome(&mut ui), json!(false), "with it shut Tab hides them");
}

/// Tab in a text field outside the palette is the field's, and leaves the keyboard on no button
/// (gs-ui `shell.rs`, the Tab block; decision 0025, section 5, amended 2026-10-14, UI-last):
/// the panels stay, and Space after it opens the palette and runs nothing.
#[test]
fn tab_from_a_field_leaves_no_button_for_space_to_press() {
    let none = Modifiers::default();
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    if !ui.shows("brush", "Hex") {
        ui.press("brush > Main");
    }
    let main = ui.state()["color.main"].clone();
    let [x, y] = crate::region::middle(ui.rect("brush > Hex"));
    ui.tap_at(x, y);
    let logged = ui.logged("").len();
    ui.tap("Tab", none);
    assert_eq!(ui.state()["layout.chrome"], json!(true), "the field's Tab");
    ui.tap("Space", none);
    assert!(ui.host.lock().shell.palette.open, "Space opens the palette");
    assert_eq!(ui.logged("").len(), logged, "and runs nothing");
    assert_eq!(ui.state()["color.main"], main, "the color stays");
}

#[test]
fn enter_in_the_hex_field_sets_the_color_and_leaves_the_recovery_offer_behind_it() {
    let dir = folder("front-hex");
    crash(&dir);
    let mut ui = launched(&dir);
    assert_eq!(buttons(&mut ui), OFFER, "the crash is offered back");
    ui.ok("primitive.sphere", json!({}));
    type_hex(&mut ui, "3366CC", "Return");
    assert_eq!(ui.state()["color.main"], json!("#3366cc"), "the field's");
    assert_eq!(
        buttons(&mut ui),
        OFFER,
        "the offer behind it is not answered"
    );
}
