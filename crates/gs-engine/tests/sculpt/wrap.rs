//! Wrap Mode through the engine (decision 0054): `brush.set` sets `tip.wrap`, `brush.save` writes
//! it and `brush.load` reads it back; a value out of range is refused at its key; a stroke with
//! symmetry is refused saying why; and Live Detail names Wrap Mode as what keeps it off.

use std::path::PathBuf;

use gs_testkit::{code, text};
use serde_json::{Value, json};

use crate::{line, sphere_session, stroke};

/// A Draw stroke across the front of the sphere with `extra` arguments.
fn draw(extra: &Value) -> Value {
    stroke("Draw", &line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 6), extra)
}

#[test]
fn brush_set_sets_wrap_mode_and_save_and_load_keep_it() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(format!("wrap-{}.toml", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    let mut s = sphere_session(None);
    let wrap = json!({ "tip": { "wrap": 3 } });
    let st = s.query("brush.set", json!({ "brush": "Draw", "settings": wrap }));
    assert_eq!(st["settings"], wrap, "{st}");
    s.query("brush.save", json!({ "path": path }));
    let file = std::fs::read_to_string(&path).unwrap();
    assert!(file.contains("wrap = 3"), "{file}");
    s.query("brush.set", json!({ "settings": { "tip": null } }));
    let loaded = s.query("brush.load", json!({ "path": path }));
    assert_eq!(loaded["settings"], wrap, "{loaded}");
    std::fs::remove_file(&path).ok();

    let e = s.err("brush.set", json!({ "settings": { "tip": { "wrap": 0 } } }));
    assert_eq!(
        (code(e.code).as_str(), e.path.as_deref()),
        ("E_ARGS_INVALID", Some("/settings/tip/wrap"))
    );
    assert!(e.message.contains("from 1 to 16"), "{}", e.message);
}

#[test]
fn a_wrapping_stroke_with_symmetry_is_refused_saying_why() {
    let mut s = sphere_session(None);
    let hash = s.hash();
    let args =
        json!({ "settings": { "tip": { "wrap": 3 } }, "symmetry": { "mirror": { "x": true } } });
    let e = s.err("brush.stroke", draw(&args));
    assert_eq!(code(e.code), "E_ARGS_INVALID", "{}", e.message);
    assert!(e.message.contains("symmetry"), "{}", e.message);
    assert_eq!(s.hash(), hash, "nothing changed");
}

#[test]
fn live_detail_names_wrap_mode_as_what_keeps_it_off() {
    let mut s = sphere_session(None);
    let hash = s.hash();
    let args = json!({ "settings": { "tip": { "wrap": 3 } }, "live_detail": { "size": 0.1 } });
    let done = s.query("brush.stroke", draw(&args));
    let ld = &done["live_detail"];
    assert_eq!(ld["refined"], json!(false), "{done}");
    let why = text(&ld["why_not"]);
    assert!(why.contains("Wrap Mode"), "{why}");
    assert!(
        why.contains("{\"tip\": {\"wrap\": 1}}"),
        "names its remedy: {why}"
    );
    assert_ne!(s.hash(), hash, "the stroke still sculpts");

    s.ok(
        "brush.set",
        json!({ "brush": "Draw", "settings": { "tip": { "wrap": 2 } } }),
    );
    let st = s.query("session.state", json!({}));
    assert_eq!(st["live_detail.blocked"], json!("brush"), "{st}");
    let why = st["live_detail.why_not"].as_str().unwrap_or_default();
    assert!(why.contains("Wrap Mode"), "{why}");
}
