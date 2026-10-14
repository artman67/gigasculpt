//! One Escape closes one thing, the one on top, as a person expects (decision 0025, section 5,
//! amended 2026-10-14): with two things open, a first Escape closes only the top one and a
//! second the next, whichever was opened over the other; Escape during a gizmo drag cancels the
//! drag and leaves open panels alone;
//! and Escape in a palette form's field shuts the palette as it always has, and nothing behind.

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::recovery::{OFFER, buttons, crash, launched};
use crate::rig::{Ui, folder, held};

/// Whether the panel `id` shows.
fn shows(ui: &mut Ui, id: &str) -> bool {
    !ui.widgets(id).is_empty()
}

#[test]
fn one_escape_shuts_the_palette_over_the_settings_window_and_a_second_the_window() {
    let none = Modifiers::default();
    let mut ui = Ui::new();
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    assert!(shows(&mut ui, "settings"), "Settings opens");
    ui.tap("Space", none);
    assert!(shows(&mut ui, "palette"), "Space opens the palette over it");
    ui.tap("Escape", none);
    assert!(
        !shows(&mut ui, "palette"),
        "Escape shuts the palette on top"
    );
    assert!(shows(&mut ui, "settings"), "and leaves Settings behind it");
    ui.tap("Escape", none);
    assert!(
        !shows(&mut ui, "settings"),
        "a second Escape shuts Settings"
    );
}

#[test]
fn one_escape_shuts_the_quick_menu_opened_over_a_menu_and_a_second_the_menu() {
    let none = Modifiers::default();
    let mut ui = Ui::new();
    crate::header::open(&mut ui, "Window");
    let [x0, y0, x1, y1] = ui.rect("viewport");
    ui.pen(f64::midpoint(x0, x1), f64::midpoint(y0, y1));
    ui.tap("Space", held("Shift"));
    assert!(shows(&mut ui, "quick"), "Shift+Space opens the quick menu");
    assert!(shows(&mut ui, "menu"), "over the Window menu, still open");
    ui.tap("Escape", none);
    assert!(
        !shows(&mut ui, "quick"),
        "Escape shuts the quick menu on top"
    );
    assert!(shows(&mut ui, "menu"), "and leaves the menu");
    ui.tap("Escape", none);
    assert!(!shows(&mut ui, "menu"), "a second Escape shuts the menu");
}

/// The other way round (UI-keys, 2026-10-14): a menu a key opens over the quick menu is on top,
/// and the order compares the two by which egui draws on top.
#[test]
fn one_escape_shuts_a_menu_a_key_opened_over_the_quick_menu_and_a_second_the_quick_menu() {
    let none = Modifiers::default();
    let mut ui = Ui::new();
    let bind = json!({
        "bind": "Ctrl+K",
        "command": "ui.menu",
        "args": { "open": "window" },
        "replace": true,
    });
    ui.ok("ui.keymap", bind);
    let [x0, y0, x1, y1] = ui.rect("viewport");
    ui.pen(f64::midpoint(x0, x1), f64::midpoint(y0, y1));
    ui.tap("Space", held("Shift"));
    assert!(shows(&mut ui, "quick"), "Shift+Space opens the quick menu");
    ui.tap("K", held("Ctrl"));
    ui.settle();
    assert!(shows(&mut ui, "menu"), "the key opens the Window menu");
    assert!(shows(&mut ui, "quick"), "over the quick menu, still open");
    ui.tap("Escape", none);
    assert!(!shows(&mut ui, "menu"), "Escape shuts the menu on top");
    assert!(shows(&mut ui, "quick"), "and leaves the quick menu");
    ui.tap("Escape", none);
    assert!(
        !shows(&mut ui, "quick"),
        "a second Escape shuts the quick menu"
    );
}

#[test]
fn escape_answers_the_recovery_question_over_the_palette_then_the_offer_then_the_palette() {
    let none = Modifiers::default();
    let dir = folder("escape-recovery");
    crash(&dir);
    let mut ui = launched(&dir);
    assert_eq!(buttons(&mut ui), OFFER, "the crash is offered back");
    ui.tap("Space", none);
    assert!(shows(&mut ui, "palette"), "Space opens the palette");
    // A search that finds nothing keeps the palette short, clear of the dialog's buttons.
    ui.input(&[Input::Text("qqqq".into())], none);
    ui.press("recovery > Discard");
    assert_eq!(
        buttons(&mut ui),
        ["Delete", "Cancel"],
        "Discard asks, on top"
    );
    ui.tap("Escape", none);
    assert_eq!(
        buttons(&mut ui),
        OFFER,
        "Escape answers the question Cancel"
    );
    assert!(shows(&mut ui, "palette"), "and leaves the palette");
    ui.tap("Escape", none);
    assert!(
        buttons(&mut ui).is_empty(),
        "a second is Later on the offer"
    );
    assert!(shows(&mut ui, "palette"), "the palette still open");
    ui.tap("Escape", none);
    assert!(!shows(&mut ui, "palette"), "a third shuts the palette");
}

/// The gizmo's pose for the window's camera, and the ends of its longest fully drawn arrow
/// clear of the panel `clear_of`, in window pixels.
fn arrow(ui: &mut Ui, clear_of: [f64; 4]) -> (Value, [[f64; 2]; 2]) {
    let (d, r) = crate::deformer_handles::described(ui);
    let inside = |p: [f64; 2]| {
        let [x0, y0, x1, y1] = clear_of;
        (x0..=x1).contains(&p[0]) && (y0..=y1).contains(&p[1])
    };
    let ends = |h: &Value| -> [[f64; 2]; 2] {
        let at = |i: usize, j: usize| h["outline"][i][j].as_f64().unwrap_or_default();
        [
            [r[0] + at(0, 0), r[1] + at(0, 1)],
            [r[0] + at(1, 0), r[1] + at(1, 1)],
        ]
    };
    // Squared, which orders them as their lengths do.
    let length = |[a, b]: &[[f64; 2]; 2]| {
        let (x, y) = (b[0] - a[0], b[1] - a[1]);
        x * x + y * y
    };
    let handles = d["handles"].as_array().cloned().unwrap_or_default();
    let grabbed = (handles.iter())
        .filter(|h| h["kind"] == "arrow" && h["pickable"] == true && h["fade"] == 1.0)
        .map(ends)
        .filter(|[a, b]| !inside(*a) && !inside(*b))
        .max_by(|a, b| length(a).total_cmp(&length(b)));
    let grabbed = grabbed.unwrap_or_else(|| panic!("no arrow to grab: {d}"));
    (d["pose"].clone(), grabbed)
}

#[test]
fn escape_during_a_gizmo_drag_cancels_it_and_leaves_settings_open() {
    let none = Modifiers::default();
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({ "radius": 0.7 }));
    ui.ok("ui.gizmo", json!({ "tool": true, "size": 75.0 }));
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    let settings = ui.rect("settings");
    let (before, [a, b]) = arrow(&mut ui, settings);
    let along = |f: f64| [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
    let start = along(0.7);
    ui.touch(start[0], start[1], false);
    for f in [0.7, 0.8, 0.9, 1.0] {
        let [x, y] = along(f);
        ui.touch(x, y, true);
    }
    assert_ne!(
        arrow(&mut ui, settings).0,
        before,
        "the drag moves the part"
    );
    ui.tap("Escape", none);
    assert!(shows(&mut ui, "settings"), "Escape leaves Settings open");
    let [x, y] = along(1.0);
    ui.touch(x, y, false);
    assert_eq!(arrow(&mut ui, settings).0, before, "and put the part back");
}

#[test]
fn escape_in_a_palette_forms_field_shuts_the_palette_as_ever_and_nothing_behind_it() {
    let none = Modifiers::default();
    let mut ui = Ui::new();
    crate::header::open(&mut ui, "Window");
    ui.press("menu > Settings");
    ui.ok("ui.palette", json!({ "form": "file.import" }));
    ui.input(&[], none);
    ui.input(&[Input::Text("model".into())], none);
    let typed = (ui.host.lock().shell.palette.form.as_ref())
        .is_some_and(|f| f.fields.iter().any(|(_, text)| text.contains("model")));
    assert!(typed, "the text is typed in the form's field");
    ui.tap("Escape", none);
    assert!(
        !shows(&mut ui, "palette"),
        "Escape shuts the palette and its form"
    );
    assert!(shows(&mut ui, "settings"), "and nothing behind it");
}
