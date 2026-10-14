//! The brush's rings hide while the view is dragged (decision 0062, row 805): from the barrel
//! button's press to its release no stroke can land, so they would only cover the model. The
//! hovering pen shows them before and after.

use gs_platform::Modifiers;

use crate::rig::{Window, pen_at, with_sphere};

/// A pen sample at (`x`, `y`) hovering, with `buttons` down, and a frame of the window.
fn hover(w: &mut Window, t: u64, [x, y]: [f32; 2], buttons: u8) {
    let mut s = pen_at(t, x, y, 0.0);
    s.buttons = buttons;
    w.pen(s);
    w.input(&[], Modifiers::default());
}

/// How many Size rings the cursor shows.
fn rings(w: &mut Window) -> usize {
    (w.cursor_lines().iter())
        .filter(|l| *l == "Size ring")
        .count()
}

#[test]
fn the_brush_rings_hide_from_a_view_drag_s_press_to_its_release() {
    let (mut w, rect) = with_sphere(None);
    let at = |dx: f32| {
        [
            f32::midpoint(rect[0], rect[2]) + dx,
            f32::midpoint(rect[1], rect[3]),
        ]
    };
    hover(&mut w, 0, at(0.0), 0);
    assert_eq!(rings(&mut w), 1, "hovering");
    hover(&mut w, 4, at(0.0), 1);
    assert_eq!(rings(&mut w), 0, "the barrel pressed, before it drags");
    for k in 1..4 {
        hover(&mut w, 4 + 4 * k, at(20.0 * k as f32), 1);
        assert_eq!(rings(&mut w), 0, "dragging the view");
    }
    hover(&mut w, 20, at(60.0), 0);
    assert_eq!(rings(&mut w), 1, "let go");
}
