//! The pen's eraser end (catalog `interface.pen-settings`, research 27's R18, decision 0036
//! section 3 as amended): `ui.navigation`'s `eraser` makes it stand for held keys while it is in
//! range, so it strokes exactly as the tip does with those keys held, through their slots. Set
//! to `tip`, as it starts, it strokes as the tip does.

use gs_platform::Modifiers;
use gs_protocol::{PenSample, PenTool};
use serde_json::{Value, json};

use crate::rig::{across, log_path, with_sphere};

/// The stroke across the sphere: with the eraser end and `ui.navigation`'s `eraser` set to
/// `eraser`, or with the tip and `held` keys. The mesh's hash after it, and the logged call.
fn stroke(eraser: Option<&str>, held: Modifiers) -> (String, Value) {
    let (mut w, rect) = with_sphere(Some(log_path("eraser")));
    if let Some(e) = eraser {
        w.ok("ui.navigation", json!({ "eraser": e }));
    }
    let tool = if eraser.is_some() {
        PenTool::Eraser
    } else {
        PenTool::Pen
    };
    w.input(&[], held);
    for s in across(rect, 6) {
        w.pen(PenSample { tool, ..s });
        w.input(&[], held);
    }
    let logged = w.logged();
    let call = logged.iter().rev().find(|l| l.0 == "brush.stroke");
    let call = call.map_or_else(|| panic!("a stroke: {logged:?}"), |l| l.1.clone());
    (w.hash(), call)
}

#[test]
fn the_eraser_end_strokes_as_the_keys_it_stands_for() {
    let none = Modifiers::default();
    // Shift: the Smooth slot's brush, as the key smooths.
    let (key, by_key) = stroke(
        None,
        Modifiers {
            shift: true,
            ..none
        },
    );
    let (end, by_end) = stroke(Some("shift"), Modifiers::default());
    assert_eq!(by_end["brush"], json!("Smooth"), "{by_end}");
    assert_eq!(end, key, "the eraser smooths as Shift does");
    assert_eq!(by_end, by_key, "the same call");
    // Alt: the brush the other way (Subtract).
    let (key, by_key) = stroke(None, Modifiers { alt: true, ..none });
    let (end, by_end) = stroke(Some("alt"), Modifiers::default());
    assert_eq!(
        (&by_end["brush"], &by_end["invert"]),
        (&json!("Draw"), &json!(true)),
        "{by_end}"
    );
    assert_eq!(end, key, "the eraser digs as Alt does");
    assert_eq!(by_end, by_key, "the same call");
    // Ctrl and Alt: the Mask slot the other way, mask erase.
    let (end, by_end) = stroke(Some("ctrl_alt"), Modifiers::default());
    let mask_erase = Modifiers {
        ctrl: true,
        alt: true,
        ..none
    };
    let (key, by_key) = stroke(None, mask_erase);
    assert_eq!(
        (&by_end["brush"], &by_end["invert"]),
        (&json!("Mask Pen"), &json!(true)),
        "{by_end}"
    );
    assert_eq!((end, by_end), (key, by_key), "as Ctrl and Alt held");
}

#[test]
fn the_eraser_end_strokes_as_the_tip_until_it_is_bound() {
    let (tip, by_tip) = stroke(None, Modifiers::default());
    let (mut w, rect) = with_sphere(None);
    let before = w.hash();
    for s in across(rect, 6) {
        w.pen(PenSample {
            tool: PenTool::Eraser,
            ..s
        });
    }
    assert_eq!(by_tip["brush"], json!("Draw"));
    assert_ne!(w.hash(), before, "it sculpts");
    assert_eq!(w.hash(), tip, "as the tip does");
    let set = w.ok("ui.navigation", json!({}));
    assert_eq!(set["eraser"], json!("tip"), "{set}");
    let state = w.ok("session.state", json!({}));
    assert_eq!(state["navigation.eraser"], json!("tip"));
}
