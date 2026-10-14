//! Curve By Pen, Accurate Curve and the profile presets through the engine (decision 0059):
//! `brush.set` sets them, `brush.save` writes them and `brush.load` reads them back, a curve
//! value out of range is refused at its key, and a stroke whose samples carry no pressure (a
//! mouse, a script) presses fully, so with the default Pen Curve it is the Edit Curve's stroke.

use std::path::PathBuf;

use gs_testkit::code;
use serde_json::{Value, json};

use crate::{line, sphere_session, stroke};

/// A light Zero Curve: a narrow peak.
fn zero() -> Value {
    json!([[0.0, 1.0], [0.25, 0.125], [1.0, 0.0]])
}

#[test]
fn brush_set_sets_curve_by_pen_accurate_curve_and_a_profile_and_save_and_load_keep_them() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(format!("curve-by-pen-{}.toml", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    let mut s = sphere_session(None);
    let pen = json!({ "points": [[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]], "smooth": true });
    let tip = json!({ "tip": {
        "profile": "flat_top", "accurate_curve": true, "curve_by_pen": true,
        "zero_curve": zero(), "pen_curve": pen
    } });
    let st = s.query("brush.set", json!({ "brush": "Draw", "settings": tip }));
    assert_eq!(st["settings"], tip, "{st}");
    s.query("brush.save", json!({ "path": path }));
    let file = std::fs::read_to_string(&path).unwrap();
    for line in [
        "profile = \"flat_top\"",
        "accurate_curve = true",
        "curve_by_pen = true",
        "zero_curve = ",
        "pen_curve",
    ] {
        assert!(file.contains(line), "{line}: {file}");
    }
    s.query("brush.set", json!({ "settings": { "tip": null } }));
    let loaded = s.query("brush.load", json!({ "path": path }));
    assert_eq!(loaded["settings"], tip, "{loaded}");
    std::fs::remove_file(&path).ok();

    for (bad, at) in [
        (
            json!({ "zero_curve": [[0.0, 1.5], [1.0, 0.0]] }),
            "/settings/tip/zero_curve",
        ),
        (
            json!({ "pen_curve": [[0.0, -0.25], [1.0, 1.0]] }),
            "/settings/tip/pen_curve",
        ),
    ] {
        let e = s.err("brush.set", json!({ "settings": { "tip": bad } }));
        assert_eq!(
            (code(e.code).as_str(), e.path.as_deref()),
            ("E_ARGS_INVALID", Some(at))
        );
        assert!(e.message.contains("from 0 to 1"), "{}", e.message);
    }
}

#[test]
fn a_stroke_without_pressure_presses_fully_and_draws_the_edit_curve() {
    let pts = line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 6);
    let by_pen = json!({ "settings": { "tip": { "curve_by_pen": true, "zero_curve": zero() } } });
    let mut hashes = Vec::new();
    for (extra, pressure) in [
        (json!({}), None),
        (by_pen.clone(), None),
        (json!({}), Some(0.25)),
        (by_pen, Some(0.25)),
    ] {
        let mut s = sphere_session(None);
        let mut args = stroke("Draw", &pts, &extra);
        if let (Some(p), Some(samples)) = (pressure, args["samples"].as_array_mut()) {
            for sample in samples {
                sample["pressure"] = json!(p);
            }
        }
        s.ok("brush.stroke", args);
        hashes.push(s.hash());
    }
    assert_eq!(
        hashes[0], hashes[1],
        "no pressure is full pressure: the Edit Curve"
    );
    assert_ne!(
        hashes[2], hashes[3],
        "a light press blends in the Zero Curve"
    );
}
