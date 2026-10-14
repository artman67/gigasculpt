//! The canvas's settings (catalog items `canvas.floor-grid`, `canvas.wireframe-view` and
//! `canvas.background`): set by logged commands, reported by `session.state`, replayed.

use gs_protocol::ErrorCode;
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::determinism::seeded;
use crate::socket::scratch as folder;
use crate::support::{EngineSession, log_records};

/// Every `session.state` value whose name starts with `prefix`.
fn state(s: &mut Session, prefix: &str) -> Vec<(String, Value)> {
    let values = s.query("session.state", json!({}));
    let values = values.as_object().cloned().unwrap_or_default();
    (values.into_iter())
        .filter(|(k, _)| k.starts_with(prefix))
        .collect()
}

/// Saving a floor preset, changing the grid and loading it returns every setting, whether the
/// floor shows too; the load is logged with the file's hash and replays to the same floor.
#[test]
fn a_floor_preset_returns_every_setting_and_replays() {
    let dir = folder("canvas-preset");
    let log = dir.join("floor.jsonl");
    let preset = dir.join("walls.toml").display().to_string();
    let mut s = seeded(0, Some(log.clone()));
    let start = state(&mut s, "view.floor");
    assert_eq!(
        start.iter().find(|(k, _)| k == "view.floor.y"),
        Some(&("view.floor.y".into(), json!(true))),
        "the y plane starts on: {start:?}"
    );
    let walls = json!({
        "x": true, "y": false, "z": true, "elevation": 0.5, "snap": true, "size": 3,
        "tiles": 8, "fill": 3, "tint": 0.75
    });
    s.ok("view.floor", walls);
    s.ok("view.display", json!({ "floor": true }));
    let saved = state(&mut s, "view.floor");
    s.ok("view.floor_save", json!({ "path": preset }));
    let again = s.err("view.floor_save", json!({ "path": preset }));
    assert_eq!(
        again.code,
        ErrorCode::Precondition,
        "a preset is kept: {again:?}"
    );
    let other = json!({
        "x": false, "y": true, "z": false, "elevation": -1, "snap": false, "size": 0,
        "tiles": 16, "fill": 0, "tint": 0
    });
    s.ok("view.floor", other);
    s.ok("view.display", json!({ "floor": false }));
    assert_ne!(state(&mut s, "view.floor"), saved, "the grid changed");
    s.ok("view.floor", json!({ "preset": preset }));
    assert_eq!(
        state(&mut s, "view.floor"),
        saved,
        "the preset returns every setting"
    );
    s.err("view.floor", json!({ "fill": 4 }));
    s.err("view.floor", json!({ "elevation": 2 }));

    let (_, records) = log_records(&log);
    let load = records.iter().find(|r| r.args.get("preset").is_some());
    let hash = load.map(|r| r.args["hash"].clone()).unwrap_or_default();
    assert!(hash.as_str().is_some_and(|h| h.len() == 64), "{hash}");
    let mut replay = seeded(0, None);
    replay.replay_log(&log);
    assert_eq!(state(&mut replay, "view.floor"), saved, "the replay");
    std::fs::remove_dir_all(dir).ok();
}

/// `view.wire` sets each part of the wire's style alone, reports it in `session.state`, refuses
/// a color that is not `"#rrggbb"` and an opacity past 1, and replays to the same style.
#[test]
fn the_wire_style_is_set_reported_and_replayed() {
    let dir = folder("canvas-wire");
    let log = dir.join("wire.jsonl");
    let mut s = seeded(0, Some(log.clone()));
    let start = state(&mut s, "view.wire.");
    let expect = |pairs: &[(&str, Value)]| -> Vec<(String, Value)> {
        let pairs = pairs
            .iter()
            .map(|(k, v)| (format!("view.wire.{k}"), v.clone()));
        pairs.collect()
    };
    let defaults = expect(&[
        ("color", json!("#14141a")),
        ("density", json!(false)),
        ("dots", json!(false)),
        ("fill", json!(false)),
        ("line", json!(true)),
        ("opacity", json!(0.5)),
    ]);
    assert_eq!(start, defaults, "the defaults");
    s.ok("view.wire", json!({ "color": "#FFffff", "opacity": 1 }));
    s.ok("view.wire", json!({ "line": false, "fill": true }));
    s.ok("view.wire", json!({ "density": true, "dots": true }));
    let set = expect(&[
        ("color", json!("#ffffff")),
        ("density", json!(true)),
        ("dots", json!(true)),
        ("fill", json!(true)),
        ("line", json!(false)),
        ("opacity", json!(1.0)),
    ]);
    assert_eq!(state(&mut s, "view.wire."), set, "every setting kept");
    for bad in [
        json!({ "color": "white" }),
        json!({ "color": "#fffff" }),
        json!({ "opacity": 2 }),
    ] {
        let e = s.err("view.wire", bad.clone());
        assert_eq!(e.code, ErrorCode::ArgsInvalid, "{bad}: {e:?}");
    }
    assert_eq!(
        state(&mut s, "view.wire."),
        set,
        "a refused call changes nothing"
    );
    let mut replay = seeded(0, None);
    replay.replay_log(&log);
    assert_eq!(state(&mut replay, "view.wire."), set, "the replay");
    std::fs::remove_dir_all(dir).ok();
}

/// `view.background` sets the background, `session.state` reports it, and a scene file keeps
/// it: opening the scene in a fresh session brings it back, while a Part file keeps none, so
/// opening one leaves the session's as it was. Changing it is no unsaved change, so an open
/// that would refuse to drop one goes ahead. A color that is not `"#rrggbb"` and a gradient
/// past 1 are refused.
#[test]
fn the_background_is_set_reported_and_saved_with_the_scene() {
    let dir = folder("canvas-background");
    let [scene, parts] = ["scene.gss", "ball.gss"].map(|f| dir.join(f).display().to_string());
    let mut s = seeded(0, None);
    s.ok("primitive.sphere", json!({}));
    let named = |pairs: [(&str, Value); 6]| -> Vec<(String, Value)> {
        let pairs = pairs.into_iter();
        (pairs.map(|(k, v)| (format!("view.background.{k}"), v))).collect()
    };
    let gray = json!("#282a2e");
    let defaults = named([
        ("border", gray.clone()),
        ("border2", gray.clone()),
        ("center", json!(0.5)),
        ("color", gray.clone()),
        ("gradient", json!(0.0)),
        ("softness", json!(0.5)),
    ]);
    assert_eq!(state(&mut s, "view.background."), defaults, "the defaults");
    s.ok("file.save", json!({ "path": scene }));
    s.ok("file.save", json!({ "path": parts, "parts": ["sphere"] }));
    let set = json!({
        "color": "#3A3A3A", "gradient": 0.25, "center": 0.75, "softness": 0,
        "border": "#000000", "border2": "#ffffff"
    });
    s.ok("view.background", set);
    let chosen = named([
        ("border", json!("#000000")),
        ("border2", json!("#ffffff")),
        ("center", json!(0.75)),
        ("color", json!("#3a3a3a")),
        ("gradient", json!(0.25)),
        ("softness", json!(0.0)),
    ]);
    assert_eq!(state(&mut s, "view.background."), chosen, "every setting");
    for bad in [json!({ "color": "#12345" }), json!({ "gradient": 2 })] {
        let e = s.err("view.background", bad.clone());
        assert_eq!(e.code, ErrorCode::ArgsInvalid, "{bad}: {e:?}");
    }
    s.ok("file.open", json!({ "path": parts }));
    assert_eq!(
        state(&mut s, "view.background."),
        chosen,
        "a Part file keeps none"
    );
    s.ok("file.save", json!({ "path": scene, "overwrite": true }));
    let mut opened = Session::new();
    opened.ok("file.open", json!({ "path": scene }));
    let kept = state(&mut opened, "view.background.");
    assert_eq!(kept, chosen, "the scene keeps it");
    let mut fresh = Session::new();
    fresh.ok("file.open", json!({ "path": parts }));
    assert_eq!(state(&mut fresh, "view.background."), defaults, "nor here");
    std::fs::remove_dir_all(dir).ok();
}
