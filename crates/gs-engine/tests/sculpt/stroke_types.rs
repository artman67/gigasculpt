//! A brush's stroke type (decision 0051, sections 5 to 7): `brush.set` with `stroke` changes the
//! brush's stroke class, which stays with the brush when brushes change, is saved by
//! `brush.save` and read back by `brush.load`; brushes whose kernel cannot run a type refuse it
//! and change nothing; and a Drag Dot drag through a camera commits what one press at its last
//! pixel makes.

use std::path::PathBuf;

use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::{line, sphere_session, stroke};

fn set(s: &mut Session, args: Value) -> Value {
    s.query("brush.set", args)
}

fn state(s: &mut Session) -> Value {
    s.query("session.state", json!({}))
}

#[test]
fn brush_set_changes_the_brushs_stroke_type_and_it_stays_with_the_brush() {
    let mut s = Session::new();
    let r = set(&mut s, json!({ "brush": "Clay", "stroke": "drag_rect" }));
    assert_eq!(r["stroke"], json!("drag_rect"), "{r}");
    assert_eq!(r["settings"]["stroke"]["class"], json!("anchored"), "{r}");
    assert_eq!(
        r["spacing"],
        Value::Null,
        "an anchored stroke has no Step: {r}"
    );
    assert_eq!(state(&mut s)["brush.stroke"], json!("drag_rect"));
    assert_eq!(
        set(&mut s, json!({ "brush": "Draw" }))["stroke"],
        json!("dots")
    );
    assert_eq!(
        set(&mut s, json!({ "brush": "Clay" }))["stroke"],
        json!("drag_rect")
    );
    let r = set(&mut s, json!({ "stroke": "freehand" }));
    assert_eq!(
        (&r["stroke"], &r["spacing"]),
        (&json!("freehand"), &json!(0.375)),
        "{r}"
    );
    let r = set(&mut s, json!({ "stroke": "dots" }));
    assert_eq!(
        (&r["stroke"], &r["spacing"]),
        (&json!("dots"), &json!(0.1_f32)),
        "{r}"
    );
    assert_eq!(r["settings"], json!({}), "Clay's own stroke again: {r}");
    let r = set(
        &mut s,
        json!({ "settings": { "stroke": { "spacing": 0.05 } } }),
    );
    let r2 = set(&mut s, json!({ "stroke": "dots" }));
    assert_eq!(
        r2["spacing"], r["spacing"],
        "picking the type shown keeps its Step"
    );
    let r = set(&mut s, json!({ "stroke": "drag_dot" }));
    assert_eq!(r["stroke"], json!("drag_dot"), "{r}");
    assert_eq!(r["settings"]["stroke"]["follow"], json!(true), "{r}");
    // A curve stroke shows no type, and takes one like any other.
    let r = set(&mut s, json!({ "brush": "Clay Line" }));
    assert_eq!(r["stroke"], Value::Null, "{r}");
    assert_eq!(
        set(&mut s, json!({ "stroke": "dots" }))["stroke"],
        json!("dots")
    );
}

#[test]
fn a_stroke_type_the_brush_cannot_run_is_refused_and_changes_nothing() {
    let mut s = Session::new();
    set(&mut s, json!({ "brush": "Clay", "stroke": "drag_rect" }));
    let before = state(&mut s);
    // A type the brush cannot run is unavailable for it, refused by the command's check (so the
    // window's dry run draws its button disabled); a name that is no type, or a class given
    // twice, is a wrong argument at `stroke`.
    let unavailable = ("E_PRECONDITION", None);
    let wrong = ("E_ARGS_INVALID", Some("/stroke"));
    for (args, want) in [
        (json!({ "brush": "Move", "stroke": "dots" }), unavailable),
        (
            json!({ "brush": "Pull Out", "stroke": "drag_dot" }),
            unavailable,
        ),
        (
            json!({ "brush": "Select Rect", "stroke": "dots" }),
            unavailable,
        ),
        (
            json!({ "brush": "Clip Curve", "stroke": "drag_rect" }),
            unavailable,
        ),
        (json!({ "stroke": "spray" }), wrong),
        (
            json!({ "stroke": "dots", "settings": { "stroke": { "class": "dab" } } }),
            wrong,
        ),
    ] {
        let err = s.err("brush.set", args.clone());
        let named = code(err.code);
        let got = (named.as_str(), err.path.as_deref());
        assert_eq!(got, want, "{args}: {}", err.message);
        assert_eq!(state(&mut s), before, "{args} changed nothing");
    }
    // A class named in the settings changes the stroke the same way.
    let r = set(
        &mut s,
        json!({ "settings": { "stroke": { "class": "dab" } } }),
    );
    assert_eq!(r["stroke"], json!("dots"), "{r}");
}

#[test]
fn a_brush_with_its_stroke_type_changed_saves_and_loads_back() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(format!("stroke-types-{}.toml", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    let mut s = sphere_session(None);
    set(&mut s, json!({ "brush": "Clay", "stroke": "drag_rect" }));
    s.query("brush.save", json!({ "path": path, "overwrite": true }));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("class = \"anchored\""), "{text}");
    assert!(
        !text.contains("spacing ="),
        "a Drag Rect brush has no spacing: {text}"
    );
    let mut back = sphere_session(None);
    let r = back.query("brush.load", json!({ "path": path }));
    assert_eq!(
        (&r["brush"], &r["stroke"]),
        (&json!("Clay"), &json!("drag_rect")),
        "{r}"
    );
    assert_eq!(r["settings"], state(&mut s)["brush.settings"], "{r}");
    // The brush strokes the same from both sessions.
    let path = line([0.0, 0.1, 0.99], [0.3, 0.2, 0.95], 5);
    for session in [&mut s, &mut back] {
        let carried = json!({ "settings": state(session)["brush.settings"] });
        session.ok("brush.stroke", stroke("Clay", &path, &carried));
    }
    assert_eq!(
        s.hash(),
        back.hash(),
        "the loaded brush strokes as the saved one"
    );
}

#[test]
fn a_drag_dot_drag_through_a_camera_commits_what_one_press_at_its_last_pixel_makes() {
    let dot = json!({ "stroke": { "class": "anchored", "follow": true } });
    let press = |pixels: &[[f32; 2]]| {
        let mut s = sphere_session(None);
        let samples: Vec<Value> = pixels.iter().map(|p| json!({ "position": p })).collect();
        let args = json!({
            "brush": "Clay", "camera": "front", "size_px": 60, "settings": dot,
            "samples": samples,
        });
        let done = s.ok("brush.stroke", args);
        (s.hash(), done.result["samples_used"].clone())
    };
    let path: Vec<[f32; 2]> = (0..8)
        .map(|i| [380.0 + 30.0 * i as f32, 470.0 + 9.0 * i as f32])
        .collect();
    let (dragged, used) = press(&path);
    assert_eq!(used, json!(8), "every sample hit the sphere");
    let (once, _) = press(&path[7..]);
    assert_eq!(
        dragged, once,
        "the drag leaves one stamp, where the pen let go"
    );
    assert_ne!(
        once,
        sphere_session(None).hash(),
        "the stamp changed the sphere"
    );
    // A sample off the part keeps the last stamp.
    let off: Vec<[f32; 2]> = path.iter().copied().chain([[5.0, 5.0]]).collect();
    assert_eq!(press(&off).0, once, "a missed sample keeps the last stamp");
}
