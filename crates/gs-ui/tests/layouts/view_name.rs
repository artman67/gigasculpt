//! The 3D view names the side it shows (review 0009, C5; decision 0017 as amended 2026-10-13):
//! a quiet label in its top corner away from the pen says Front, Back, Left, Right, Top or
//! Bottom while the camera looks along one, then Perspective or Orthographic; between them it
//! says only the projection. `view.get`'s `looks_from` names the same side, from the same
//! function.

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::rig::{Ui, folder};

/// What the label in the 3D view's corner says, and where it shows (pixels).
fn label(ui: &mut Ui) -> (String, [f64; 4]) {
    let widgets = ui.widgets("viewport");
    let named = |w: &&Value| {
        let text = w["label"].as_str().unwrap_or_default();
        text.ends_with("Perspective") || text.ends_with("Orthographic")
    };
    let found = widgets.iter().find(named);
    let found = found.unwrap_or_else(|| panic!("the 3D view names its view: {widgets:?}"));
    let rect = std::array::from_fn(|i| found["rect"][i].as_f64().unwrap_or_default());
    (
        found["label"].as_str().unwrap_or_default().to_string(),
        rect,
    )
}

/// The label's words and `view.get`'s side for the window's view.
fn named(ui: &mut Ui) -> (String, Value) {
    let side = ui.ok("view.get", json!({}))["looks_from"].clone();
    (label(ui).0, side)
}

fn says(words: &str, side: &str) -> (String, Value) {
    (words.to_string(), json!(side))
}

/// The pen hovering over the 3D view's middle, where the view keys run.
fn over_view(ui: &mut Ui) {
    let [x, y] = ui.middle();
    ui.pen(x, y);
}

/// Review 0009, C5, the way it found it: start the window, Add > Sphere, View > Look along Z.
/// The window starts at the front and says so; Look along Z there turns to the back and the
/// corner says Back; a click on the figure's Z end turns to the front again, and V Z to the
/// back, each named as `view.get` names it.
#[test]
fn look_along_z_from_the_start_view_says_back_and_the_figure_s_z_end_says_front() {
    let dir = folder("view-name");
    let mut ui = Ui::launch(&dir, &dir.join("state"));
    ui.settle();
    assert_eq!(named(&mut ui), says("Front · Perspective", "front"));
    crate::header::open(&mut ui, "Add");
    ui.press("menu > Sphere");
    assert_eq!(named(&mut ui), says("Front · Perspective", "front"));
    crate::header::open(&mut ui, "View");
    ui.press("menu > Z");
    assert_eq!(named(&mut ui), says("Back · Perspective", "back"));
    ui.press("figure > Look from front");
    assert_eq!(named(&mut ui), says("Front · Perspective", "front"));
    over_view(&mut ui);
    for key in ["V", "Z"] {
        ui.tap(key, Modifiers::default());
    }
    assert_eq!(named(&mut ui), says("Back · Perspective", "back"));
}

/// An orbit drag on empty canvas leaves no side named, here or in `view.get`; P switches the
/// projection's word, at a side and between them.
#[test]
fn an_orbit_names_no_side_and_p_switches_the_projection_s_word() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    over_view(&mut ui);
    ui.tap("P", Modifiers::default());
    assert_eq!(named(&mut ui), says("Front · Orthographic", "front"));
    // Off the sphere, at the view's left: the sphere fills the middle of a wide view.
    let [left, top, _, bottom] = ui.rect("viewport");
    let (x, y) = (left + 40.0, f64::midpoint(top, bottom));
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    for step in 1..=4 {
        ui.touch(x + 30.0 * f64::from(step), y, true);
    }
    ui.touch(x + 120.0, y, false);
    assert_eq!(named(&mut ui), ("Orthographic".to_string(), Value::Null));
    over_view(&mut ui);
    ui.tap("P", Modifiers::default());
    assert_eq!(named(&mut ui), ("Perspective".to_string(), Value::Null));
}

/// The label sits in the 3D view's top corner away from the pen, inside the view and clear of
/// the corner figure and its Zoom and Pan buttons, at the smallest and the largest interface
/// scale, for either hand.
#[test]
fn the_view_s_name_sits_in_the_top_corner_away_from_the_pen_at_every_scale() {
    let mut ui = Ui::new();
    ui.ok("primitive.sphere", json!({}));
    let apart =
        |a: [f64; 4], b: [f64; 4]| a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1];
    for hand in ["right", "left"] {
        for scale in [0.5, 3.0] {
            ui.ok("ui.layout", json!({ "hand": hand, "scale": scale }));
            ui.settle();
            let (words, at) = label(&mut ui);
            let view = ui.rect("viewport");
            let case = format!("{hand} hand at {scale}: {words} at {at:?} in {view:?}");
            let inside = view[0] <= at[0] && at[2] <= view[2] && view[1] <= at[1];
            assert!(inside && at[3] <= view[3], "inside the view, {case}");
            let near = at[1] - view[1] <= 0.1 * (view[3] - view[1]);
            let away = match hand {
                "right" => at[0] - view[0] < view[2] - at[2],
                _ => view[2] - at[2] < at[0] - view[0],
            };
            assert!(near && away, "the top corner away from the pen, {case}");
            for id in ["figure", "pads"] {
                if let Some(p) = crate::header::panel(&mut ui, id) {
                    let r = std::array::from_fn(|i| p["rect"][i].as_f64().unwrap_or_default());
                    assert!(apart(at, r), "clear of the {id} at {r:?}, {case}");
                }
            }
        }
    }
}
