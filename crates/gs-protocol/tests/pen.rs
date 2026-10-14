//! `PenSample`'s wire form: only the position is required, and what is left out means a pen
//! hovering in range with nothing pressed, that presses fully when it touches (decision 0036
//! section 3 as amended: a sample without pressure means 1, as `brush.stroke`'s does).

use gs_protocol::{PenSample, PenTool};
use serde_json::json;

#[test]
fn left_out_fields_mean_a_hovering_pen_in_range() {
    let sample: PenSample =
        serde_json::from_value(json!({ "x": 3.5, "y": -2.0 })).unwrap_or_else(|e| panic!("{e}"));
    let pressed = PenSample {
        pressure: 1.0,
        ..PenSample::hover(0, 3.5, -2.0)
    };
    assert_eq!(sample, pressed, "a pressure left out is full pressure");
    assert!(sample.in_proximity, "in range unless said otherwise");
    assert_eq!(
        PenSample::hover(0, 3.5, -2.0).pressure,
        0.0,
        "a hover built by the platform reports none"
    );
    let full = PenSample {
        t_ns: 7,
        pressure: 0.5,
        tilt_x: -3.0,
        tilt_y: 4.0,
        tool: PenTool::Eraser,
        buttons: 2,
        contact: true,
        in_proximity: false,
        ..PenSample::hover(0, 1.0, 2.0)
    };
    let text = serde_json::to_value(full).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(text["tool"], json!("eraser"), "{text}");
    let back: PenSample = serde_json::from_value(text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back, full, "every field survives the round trip");
}

#[test]
fn the_position_is_required_and_unknown_fields_are_refused() {
    let missing = serde_json::from_value::<PenSample>(json!({ "x": 1.0 }));
    assert!(missing.is_err(), "y is required");
    let typo = serde_json::from_value::<PenSample>(json!({ "x": 1.0, "y": 1.0, "presure": 1 }));
    assert!(typo.is_err(), "a misspelled field is an error, not ignored");
    let schema = serde_json::to_value(schemars::schema_for!(PenSample)).unwrap_or_default();
    assert_eq!(schema["required"], json!(["x", "y"]), "{schema}");
}
