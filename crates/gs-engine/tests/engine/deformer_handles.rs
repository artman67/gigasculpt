//! `gizmo.describe` reports the open deformer's handles (decision 0041, amended 2026-10-13 for
//! `DF7c`, section 9): its box as eight corners in the scene and each handle's name, kind, the
//! argument it drives and its outline in pixels, from the function the window draws them with,
//! and `at` names a deformer handle before a gizmo handle.

use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::support::{EngineSession, human};

/// The gizmo seen from the front, a little right and above, on an 800 by 600 image.
fn describe(s: &mut Session, at: Option<[f32; 2]>) -> Value {
    let camera = json!({
        "azimuth_deg": 20,
        "elevation_deg": 10,
        "target": [0, 0, 0],
        "distance": 5,
    });
    let mut args = json!({ "camera": camera, "viewport": [800, 600] });
    if let Some(at) = at {
        args["at"] = json!(at);
    }
    s.query("gizmo.describe", args)
}

fn number(v: &Value) -> f32 {
    v.as_f64().unwrap_or(f64::NAN) as f32
}

fn xyz(v: &Value) -> [f32; 3] {
    [0, 1, 2].map(|i| number(&v[i]))
}

#[test]
fn gizmo_describe_reports_the_open_deformers_box_and_cone() {
    let mut s = human();
    s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
    let shut = describe(&mut s, None);
    assert!(shut.get("deformer").is_none(), "none open: {shut}");

    let bend = json!({
        "part": "sphere",
        "frame": { "origin": [0, 0, 0], "axes": [0, 0, 0, 1] },
        "op": { "bend": { "axis": "x", "toward": "y", "degrees": 0 } },
    });
    let open = json!({ "phase": "open", "transforms": [bend] });
    success(s.as_human("gizmo.deformer", open), "open");
    let d = describe(&mut s, None);
    let deformer = &d["deformer"];

    // The box the open measured, at its eight corners: the low one first, the high one last.
    let state = s.query("session.state", json!({}));
    let measured = &state["deformer.transforms"][0]["box"];
    let boxes = deformer["boxes"].as_array().cloned().unwrap_or_default();
    assert_eq!(boxes.len(), 1, "no symmetry, no repeat: {deformer}");
    let corners = boxes[0].as_array().cloned().unwrap_or_default();
    assert_eq!(corners.len(), 8);
    for (corner, end) in [(&corners[0], "lo"), (&corners[7], "hi")] {
        let (got, want) = (xyz(corner), xyz(&measured[end]));
        let off = (0..3).map(|i| (got[i] - want[i]).abs()).fold(0.0, f32::max);
        assert!(off < 1e-5, "{end}: {got:?} against {want:?}");
    }

    // One cone, on X's face toward the camera, driving the bend's angle.
    let handles = deformer["handles"].as_array().cloned().unwrap_or_default();
    assert_eq!(handles.len(), 1, "{deformer}");
    let cone = &handles[0];
    assert_eq!(
        [&cone["name"], &cone["kind"], &cone["drives"], &cone["axis"]],
        [
            &json!("cone_x"),
            &json!("cone"),
            &json!("/bend/degrees"),
            &json!("x")
        ]
    );
    assert_eq!(
        (number(&cone["fade"]), &cone["pickable"]),
        (1.0, &json!(true))
    );
    let outline = cone["outline"].as_array().cloned().unwrap_or_default();
    assert_eq!(outline.len(), 3, "tip and the base's two ends: {cone}");

    // A press on the middle of its base grabs it.
    let [a, b] = [&outline[1], &outline[2]].map(|p| [number(&p[0]), number(&p[1])]);
    let base = [f32::midpoint(a[0], b[0]), f32::midpoint(a[1], b[1])];
    let picked = describe(&mut s, Some(base));
    assert_eq!(picked["at"], json!("cone_x"), "{base:?}");
}
