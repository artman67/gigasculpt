//! Live Detail and the automasks (decision 0021, section 3, amended 2026-10-12): a stroke whose
//! brush has the backface, face group or topology automask on runs without refining, adds no
//! triangles, and says why, naming the automask and its remedy; `session.state` names it in
//! `live_detail.blocked`, the value the window dims its switch from. The cavity automask
//! refines.

use gs_testkit::{Session, text};
use serde_json::{Value, json};

use crate::support::agent;

/// A session with the part "sphere", 3,456 quads.
fn sphere() -> Session {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    s
}

/// The sphere's face count.
fn faces(s: &mut Session) -> u64 {
    let stats = s.query("mesh.stats", json!({}));
    stats["parts"][0]["faces"].as_u64().unwrap_or_default()
}

/// A refining Draw stroke across the front of the sphere with the brush's `automask` patched.
fn refining(automask: &Value) -> Value {
    let samples: Vec<Value> = (0..12)
        .map(|i| {
            let t = i as f32 / 11.0;
            json!({ "position": [-0.4 + 0.8 * t, 0.1 + 0.1 * t, 0.92 - 0.02 * t] })
        })
        .collect();
    json!({
        "brush": "Draw",
        "samples": samples,
        "live_detail": { "size": 0.1 },
        "settings": { "automask": automask },
    })
}

/// "A brush with Backface on adds no Live Detail triangles", and its answer says why.
#[test]
fn a_brush_with_backface_on_adds_no_live_detail_triangles_and_says_why() {
    for (automask, name, field) in [
        (json!({ "backface": 0.5 }), "backface", "backface"),
        (json!({ "face_group": 1.0 }), "face group", "face_group"),
        (json!({ "topology": 1.0 }), "topology", "topology"),
    ] {
        let mut s = sphere();
        let (before, hash) = (faces(&mut s), s.hash());
        let done = s.query("brush.stroke", refining(&automask));
        let ld = &done["live_detail"];
        assert_eq!(ld["refined"], json!(false), "{automask}: {done}");
        let why = text(&ld["why_not"]);
        assert!(
            why.contains(&format!("{name} automask")),
            "{automask}: {why}"
        );
        let remedy = format!("{{\"automask\": {{\"{field}\": 0}}}}");
        assert!(why.contains(&remedy), "{automask} names its remedy: {why}");
        assert_eq!(faces(&mut s), before, "{automask}: no triangle added");
        assert_ne!(s.hash(), hash, "{automask}: the stroke still sculpts");
    }
    let mut s = sphere();
    let before = faces(&mut s);
    let done = s.query("brush.stroke", refining(&json!({ "cavity": 1.0 })));
    assert_eq!(done["live_detail"]["refined"], json!(true), "{done}");
    assert!(faces(&mut s) > before, "the cavity automask refines");
}

/// `session.state` names the automask in `live_detail.blocked`, with `why_not`, as long as the
/// session's brush has it on; the cavity automask blocks nothing.
#[test]
fn session_state_names_the_automask_that_keeps_live_detail_off() {
    let mut s = sphere();
    for (automask, blocked) in [
        (json!({ "backface": 1.0 }), json!("backface")),
        (
            json!({ "backface": 0.0, "face_group": 0.5 }),
            json!("face_group"),
        ),
        (
            json!({ "face_group": 0.0, "topology": 2.0 }),
            json!("topology"),
        ),
        (json!({ "topology": 0.0, "cavity": -1.0 }), Value::Null),
    ] {
        s.ok("brush.set", json!({ "settings": { "automask": automask } }));
        let st = s.query("session.state", json!({}));
        assert_eq!(st["live_detail.blocked"], blocked, "{automask}");
        let eligible = json!(blocked.is_null());
        assert_eq!(st["live_detail.eligible"], eligible, "{automask}");
        let why = st["live_detail.why_not"].as_str().unwrap_or_default();
        assert_eq!(
            why.contains("automask"),
            !blocked.is_null(),
            "{automask}: {why}"
        );
    }
}
