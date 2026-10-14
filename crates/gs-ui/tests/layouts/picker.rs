//! Brush tray > Paint's picker and the pick (decision 0039 section 11, as amended 2026-10-12 in
//! step P8b): the wheel and the gray bar set exact bytes, which the hex field and the sliders
//! agree with; a drag on the wheel sends at most one `color.set` a frame and no undo step; a
//! drag from Main onto a painted part sets Main to the color stored there, and with Alt to the
//! pixel the view drew; C picks the color under the pen into the edited swatch; and Escape
//! during a swatch's drag sends nothing.

use std::f64::consts::FRAC_1_SQRT_2;

use gs_platform::Modifiers;
use serde_json::{Value, json};

use crate::paint::{BLUE, RED, colors, hint, pixel_at, type_hex, undos};
use crate::region::middle;
use crate::rig::{Ui, held};

/// The middle of the hue ring, as a fraction of the wheel's half width from its middle: the
/// ring is the wheel's outer fifth.
const RING: f64 = 0.9;

/// The color the tests paint their sphere: a blue that shading does not give back.
const PAINT: &str = "#3366cc";

/// The bytes of `#rrggbb`.
fn bytes(hex: &str) -> [u8; 3] {
    let channel = |i: usize| u8::from_str_radix(hex.get(1 + 2 * i..3 + 2 * i).unwrap_or(""), 16);
    [0, 1, 2].map(|i| channel(i).unwrap_or_default())
}

/// The point `f` of the way from `a` to `b`.
fn along(f: f64, a: f64, b: f64) -> f64 {
    a + f * (b - a)
}

/// Every reading of the edited color says `hex`: the palette's Main, Main's hover, the three
/// sliders and the picker's three areas, each running `color.set` with it.
fn agrees(ui: &mut Ui, hex: &str) {
    assert_eq!(colors(ui).0, json!(hex), "the palette's Main");
    assert!(hint(ui, "Main").contains(hex), "Main says {hex}");
    let set = json!({ "command": "color.set", "args": { "main": hex } });
    for label in [
        "Red",
        "Green",
        "Blue",
        "Hue",
        "Saturation and value",
        "Gray",
    ] {
        assert_eq!(ui.seen("brush", label)["command"], set, "{label}");
    }
}

#[test]
fn the_wheel_and_the_gray_bar_set_exact_bytes_the_hex_field_and_the_sliders_agree_with() {
    let mut ui = Ui::new();
    ui.ok("color.set", json!({ "main": RED }));
    assert!(!ui.shows("brush", "Hue"), "the picker starts shut");
    ui.press("brush > Main");
    assert!(
        ui.shows("brush", "Hue"),
        "a tap on the edited swatch opens it"
    );
    let undone = undos(&mut ui);
    // 45 degrees clockwise from red at the top, at red's full saturation and value.
    let wheel = ui.rect("brush > Hue");
    let ([x, y], r) = (middle(wheel), RING * (wheel[2] - wheel[0]) / 2.0);
    ui.tap_at(x + r * FRAC_1_SQRT_2, y - r * FRAC_1_SQRT_2);
    agrees(&mut ui, "#ffbf00");
    // Saturation 0.4 from the left and value 0.8 from the foot, at that hue: (0.8, 0.72, 0.48).
    let square = ui.rect("brush > Saturation and value");
    let (sx, sy) = (
        along(0.4, square[0], square[2]),
        along(0.2, square[1], square[3]),
    );
    ui.tap_at(sx, sy);
    agrees(&mut ui, "#ccb87a");
    for (f, gray) in [(0.25, "#404040"), (0.75, "#bfbfbf")] {
        let bar = ui.rect("brush > Gray");
        ui.tap_at(along(f, bar[0], bar[2]), middle(bar)[1]);
        agrees(&mut ui, gray);
    }
    // And the other way: a typed color moves the wheel.
    type_hex(&mut ui, PAINT, "Return");
    agrees(&mut ui, PAINT);
    assert_eq!(undos(&mut ui), undone, "no color is an undo step");
    ui.press("brush > Main");
    assert!(!ui.shows("brush", "Hue"), "a second tap shuts it");
}

#[test]
fn a_drag_on_the_wheel_sends_at_most_one_color_set_a_frame_and_no_undo_step() {
    let mut ui = Ui::new();
    ui.ok("color.set", json!({ "main": RED }));
    ui.press("brush > Main");
    let square = ui.rect("brush > Saturation and value");
    let point = |f: f64| {
        [
            along(f, square[0], square[2]),
            along(f, square[1], square[3]),
        ]
    };
    let (mut sets, undone) = (ui.logged("color.set").len(), undos(&mut ui));
    let [x, y] = point(0.15);
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    sets += 1;
    assert_eq!(
        ui.logged("color.set").len(),
        sets,
        "the press sets the color"
    );
    for frame in 0..3 {
        // Four samples in one frame, as a fast pen sends them.
        let samples: Vec<Value> = (1..=4)
            .map(|k| {
                let [x, y] = point(0.15 + 0.2 * f64::from(frame) + 0.05 * f64::from(k));
                json!({ "x": x, "y": y, "contact": true, "pressure": 0.5 })
            })
            .collect();
        ui.ok("pen.input", json!({ "samples": samples }));
        ui.settle();
        sets += 1;
        assert_eq!(
            ui.logged("color.set").len(),
            sets,
            "frame {frame}: one call"
        );
    }
    let [x, y] = point(0.75);
    ui.touch(x, y, false);
    assert_eq!(
        ui.logged("color.set").len(),
        sets,
        "letting go sends no more"
    );
    // Red at saturation 0.75 and value 0.25: (0.25, 0.0625, 0.0625).
    assert_eq!(colors(&mut ui).0, json!("#401010"));
    assert_eq!(undos(&mut ui), undone, "no undo step");
}

/// A sphere painted [`PAINT`] all over and framed in the 3D view, Main red and Secondary blue.
fn painted(ui: &mut Ui) {
    ui.ok("primitive.sphere", json!({}));
    ui.ok("paint.fill", json!({ "color": PAINT }));
    let frame = json!({ "view": "main", "target": "part", "part": "sphere" });
    ui.ok("view.frame", frame);
    ui.ok("color.set", json!({ "main": RED, "secondary": BLUE }));
}

/// The pen pressed on Main and dragged to `to` in four steps, still down.
fn drag_from_main(ui: &mut Ui, to: [f64; 2]) {
    let [x, y] = middle(ui.rect("brush > Main"));
    ui.touch(x, y, false);
    ui.touch(x, y, true);
    for step in 1..=4 {
        let f = f64::from(step) / 4.0;
        ui.touch(along(f, x, to[0]), along(f, y, to[1]), true);
    }
}

/// A point of the 3D view off every part: near its left edge, at its middle's height.
fn off_the_model(ui: &mut Ui) -> [f64; 2] {
    let view = ui.rect("viewport");
    [view[0] + 30.0, middle(view)[1]]
}

#[test]
fn a_drag_from_main_onto_a_painted_part_sets_main_to_the_color_stored_there() {
    let mut ui = Ui::new();
    painted(&mut ui);
    let sets = ui.logged("color.set").len();
    let to = ui.middle();
    drag_from_main(&mut ui, to);
    assert!(
        hint(&mut ui, "Main").contains(PAINT),
        "Main follows the pen"
    );
    ui.touch(to[0], to[1], false);
    assert_eq!(colors(&mut ui), (json!(PAINT), json!(BLUE)));
    assert_eq!(
        ui.logged("color.set").len(),
        sets + 1,
        "one call, on release"
    );
    // Off every part the swatch keeps its color.
    ui.ok("color.set", json!({ "main": RED }));
    let off = off_the_model(&mut ui);
    drag_from_main(&mut ui, off);
    ui.touch(off[0], off[1], false);
    assert_eq!(colors(&mut ui).0, json!(RED));
    assert_eq!(
        ui.logged("color.set").len(),
        sets + 2,
        "nothing sent off the model"
    );
}

#[test]
fn with_alt_a_drag_from_main_sets_it_to_the_pixel_the_view_drew() {
    let mut ui = Ui::with_looks();
    painted(&mut ui);
    let to = ui.middle();
    // As the settled frame drew it, with the pen elsewhere.
    let drawn = pixel_at(&mut ui, to);
    ui.input(&[], held("Alt"));
    drag_from_main(&mut ui, to);
    ui.touch(to[0], to[1], false);
    ui.input(&[], Modifiers::default());
    let main = colors(&mut ui).0;
    let got = bytes(main.as_str().unwrap_or_default());
    let off = (0..3).map(|k| got[k].abs_diff(drawn[k])).max();
    assert!(off <= Some(2), "Main {got:?}, drawn {drawn:?}");
    let stored = bytes(PAINT);
    let shaded = (0..3).any(|k| got[k].abs_diff(stored[k]) > 8);
    assert!(
        shaded,
        "the shaded color, not the stored {stored:?}: {got:?}"
    );
}

#[test]
fn c_picks_the_color_under_the_pen_into_the_edited_swatch() {
    let mut ui = Ui::new();
    painted(&mut ui);
    let (to, none) = (ui.middle(), Modifiers::default());
    ui.pen(to[0], to[1]);
    ui.tap("C", none);
    assert_eq!(colors(&mut ui), (json!(PAINT), json!(BLUE)), "into Main");
    ui.press("brush > Secondary");
    ui.pen(to[0], to[1]);
    ui.tap("C", none);
    let both = (json!(PAINT), json!(PAINT));
    assert_eq!(colors(&mut ui), both, "into Secondary once it is edited");
    let sets = ui.logged("color.set").len();
    let off = off_the_model(&mut ui);
    ui.pen(off[0], off[1]);
    ui.tap("C", none);
    assert_eq!(
        ui.logged("color.set").len(),
        sets,
        "off every part, nothing"
    );
}

#[test]
fn escape_during_a_swatch_s_drag_sends_nothing() {
    let mut ui = Ui::new();
    painted(&mut ui);
    let sets = ui.logged("color.set").len();
    let to = ui.middle();
    drag_from_main(&mut ui, to);
    assert!(
        hint(&mut ui, "Main").contains(PAINT),
        "Main follows the pen"
    );
    ui.tap("Escape", Modifiers::default());
    ui.touch(to[0], to[1], false);
    assert_eq!(colors(&mut ui).0, json!(RED));
    assert_eq!(ui.logged("color.set").len(), sets, "Escape sends nothing");
    assert!(
        hint(&mut ui, "Main").contains(RED),
        "Main shows its own again"
    );
}
