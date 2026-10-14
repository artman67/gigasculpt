//! One settings model (decision 0018): each brush keeps its own patch of changed settings in
//! the session, `session.state` shows it, a stroke carries it and pins its preset's hash, and
//! the log replays the stroke alone; a replay whose brush changed fails on that line, unless the
//! log carries the preset it was recorded with (decision 0042).

use gs_engine::EngineConfig;
use gs_protocol::Role;
use gs_testkit::{Session, code, success};
use serde_json::{Value, json};

use crate::{line, log_path, sphere_session, stroke, stroke_lines};

fn set(s: &mut Session, args: Value) -> Value {
    s.query("brush.set", args)
}

fn state(s: &mut Session) -> Value {
    s.query("session.state", json!({}))
}

#[test]
fn each_brush_keeps_its_own_settings_when_brushes_change() {
    // The catalog's check (brush-settings.size-falloff-strength): Clay Build at 40, Smooth at
    // 15, back and forth, each keeps its own; so does the Brush Modifier, per brush.
    let mut s = Session::new();
    set(&mut s, json!({ "brush": "Clay Build", "strength": 0.4 }));
    set(&mut s, json!({ "brush": "Smooth", "strength": 0.15 }));
    set(&mut s, json!({ "brush": "Draw", "modifier": 30 }));
    set(
        &mut s,
        json!({ "brush": "Clay", "modifier": -20, "falloff": 15 }),
    );
    for (brush, strength, modifier) in [
        ("Clay Build", 0.4, 0.0),
        ("Smooth", 0.15, 0.0),
        ("Draw", 0.4, 30.0),
        ("Clay", 0.25, -20.0),
        ("Clay Build", 0.4, 0.0),
    ] {
        let now = set(&mut s, json!({ "brush": brush }));
        let st = state(&mut s);
        assert_eq!(st["brush"], json!(brush));
        let f = |v: &Value| v.as_f64().map(|x| x as f32);
        assert_eq!(f(&now["strength"]), Some(strength), "{brush}'s strength");
        assert_eq!(f(&st["brush.strength"]), Some(strength), "{brush}, state");
        assert_eq!(
            f(&st["brush.modifier"]),
            Some(modifier),
            "{brush}'s modifier"
        );
    }
    let clay = set(&mut s, json!({ "brush": "Clay" }));
    assert_eq!(
        clay["settings"],
        json!({ "modifier": -20.0, "tip": { "focal": 15.0 } }),
        "the patch, in the preset's own names"
    );
    // Any setting, by its preset name; null takes it back to the preset's own.
    let more = json!({ "settings": { "stroke": { "spacing": 0.3 }, "tip": { "focal": null } } });
    let after = set(&mut s, more);
    assert_eq!(
        after["settings"],
        json!({ "modifier": -20.0, "stroke": { "spacing": 0.3 } }),
        "merged, and the Falloff dropped"
    );
    assert_eq!(after["falloff"], json!(0.0), "Clay's own Falloff again");
    let described = s.query("brush.describe", json!({ "brush": "Clay" }));
    assert_eq!(
        described["changed"], after["settings"],
        "describe shows the patch"
    );
    assert_eq!(
        described["settings"]["stroke"]["spacing"]
            .as_f64()
            .map(|x| x as f32),
        Some(0.3),
        "and resolves it over the preset"
    );
    assert_eq!(state(&mut s)["brush.settings"], after["settings"]);
}

/// The brush's ⋯ in the tray (decision 0060, section 6): Reset to defaults drops every change
/// made to the brush in this session, and Copy settings to… gives another brush this one's
/// changes with `for`, which chooses nothing.
#[test]
fn reset_puts_a_brush_back_as_it_ships_and_for_sets_another_without_choosing_it() {
    let mut s = Session::new();
    let spacing = json!({ "stroke": { "spacing": 0.3 } });
    set(
        &mut s,
        json!({ "brush": "Clay", "strength": 0.6, "settings": spacing }),
    );
    let copied = set(&mut s, json!({ "for": "Draw", "settings": spacing }));
    assert_eq!(copied["brush"], "Clay", "the pen keeps its brush");
    assert_eq!(set(&mut s, json!({ "brush": "Draw" }))["settings"], spacing);
    let back = set(&mut s, json!({ "brush": "Clay", "reset": true }));
    assert_eq!(back["settings"], json!({}), "every change dropped");
    assert_eq!(back["strength"], json!(0.25), "Clay's own strength");
    let err = s.err("brush.set", json!({ "for": "Nothing", "reset": true }));
    assert_eq!(err.path.as_deref(), Some("/for"), "{}", err.message);
}

#[test]
fn bad_settings_are_refused_with_their_key_and_change_nothing() {
    let mut s = Session::new();
    set(&mut s, json!({ "brush": "Clay", "strength": 0.6 }));
    let before = state(&mut s);
    for (args, path) in [
        (
            json!({ "settings": { "tip": { "focal": 200 } } }),
            "/settings/tip/focal",
        ),
        // Review 0004, m2: a wrong type or shape is refused at its key in the settings.
        (
            json!({ "settings": { "tip": { "bogus": 1 } } }),
            "/settings/tip/bogus",
        ),
        (
            json!({ "settings": { "strength": "high" } }),
            "/settings/strength",
        ),
        (
            json!({ "settings": { "kernel": { "terms": [{ "gain": 1 }] } } }),
            "/settings/kernel",
        ),
        (json!({ "settings": { "name": "Other" } }), "/settings/name"),
        (
            json!({ "strength": 0.2, "settings": { "strength": 0.3 } }),
            "/strength",
        ),
        (json!({ "brush": "Smooth", "modifier": 30 }), "/modifier"),
        (
            json!({ "settings": { "modifier": 30 }, "brush": "Trim" }),
            "/settings/modifier",
        ),
        (
            json!({ "global_pressure": { "tilt": [[0, 1], [1, 1]] } }),
            "/global_pressure/tilt",
        ),
        (
            json!({ "global_pressure": { "size": [[0, 1]] } }),
            "/global_pressure",
        ),
        // Review 0004, M3: a curve point above 1 (a 0 to 100 scale) is refused by name.
        (
            json!({ "settings": { "pressure": { "strength": [[0, 100], [1, 100]] } } }),
            "/settings/pressure/strength",
        ),
        (
            json!({ "global_pressure": { "size": [[0, 2], [1, 2]] } }),
            "/global_pressure",
        ),
        // Review 0004, S1: an Offset gain past decision 0018's -4 to 4 is refused by name.
        (
            json!({ "settings": { "kernel": { "terms": [{ "dir": "area_normal", "gain": 100 }] } } }),
            "/settings/kernel/terms",
        ),
    ] {
        let err = s.err("brush.set", args.clone());
        assert_eq!(code(err.code), "E_ARGS_INVALID", "{args}");
        assert_eq!(err.path.as_deref(), Some(path), "{args}: {}", err.message);
        assert!(!err.message.contains("does not parse"), "{}", err.message);
        assert_eq!(state(&mut s), before, "{args} changed nothing");
    }
}

#[test]
fn a_patch_round_trips_through_brush_set_session_state_and_the_log() {
    let log = log_path("patch");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let patch = json!({
        "strength": 0.4,
        "modifier": 35,
        "tip": { "focal": -30, "hardness": 0.2 },
        "stroke": { "spacing": 0.05 },
    });
    set(&mut s, json!({ "brush": "Draw", "settings": patch }));
    let carried = state(&mut s)["brush.settings"].clone();
    let path = line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 6);
    s.ok(
        "brush.stroke",
        stroke("Draw", &path, &json!({ "settings": carried })),
    );
    let with_patch = s.hash();
    // The same settings as a stroke's shortcuts and keys give the same bits.
    s.ok("history.undo", json!({}));
    let split = json!({
        "strength": 0.4, "modifier": 35, "falloff": -30,
        "settings": { "tip": { "hardness": 0.2 }, "stroke": { "spacing": 0.05 } },
    });
    s.ok("brush.stroke", stroke("Draw", &path, &split));
    assert_eq!(s.hash(), with_patch, "shortcuts are the same settings");
    let lines = stroke_lines(&log);
    let first = &lines[0];
    assert_eq!(
        first.args["settings"], carried,
        "the log keeps the patch inline"
    );
    let described = s.query("brush.describe", json!({ "brush": "Draw" }));
    assert_eq!(
        first.args["preset_hash"], described["preset_hash"],
        "and pins the preset's hash"
    );
    // A fresh session with other settings replays the stroke alone.
    let mut replay = Session::new();
    replay.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    set(
        &mut replay,
        json!({ "brush": "Draw", "strength": 1.0, "falloff": 80 }),
    );
    let request = first.request().unwrap_or_else(|e| panic!("{e}"));
    success(replay.request(request), "the logged stroke");
    assert_eq!(Some(replay.hash()), first.hash, "replays exactly");
    std::fs::remove_file(&log).ok();
}

#[test]
fn a_stroke_whose_brush_changed_is_refused_on_its_pinned_hash() {
    let mut s = sphere_session(None);
    let before = s.hash();
    let path = line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 4);
    let stale = "0".repeat(64);
    let err = s.err(
        "brush.stroke",
        stroke("Clay Build", &path, &json!({ "preset_hash": stale })),
    );
    assert_eq!(code(err.code), "E_PRECONDITION");
    assert!(
        err.message.contains("Clay Build") && err.message.contains(&stale),
        "names the brush and the hash: {}",
        err.message
    );
    assert_eq!(s.hash(), before, "nothing changed");
    let described = s.query("brush.describe", json!({ "brush": "Clay Build" }));
    let pinned = json!({ "preset_hash": described["preset_hash"] });
    s.ok("brush.stroke", stroke("Clay Build", &path, &pinned));
    assert_ne!(s.hash(), before, "the brush's own hash is accepted");
}

#[test]
fn a_brush_on_the_global_pressure_curves_logs_them_and_replays_alone() {
    let log = log_path("global");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let curve = json!([[0, 0.2], [1, 0.9]]);
    let global = set(&mut s, json!({ "global_pressure": { "size": curve } }));
    assert_eq!(
        global["global_pressure"]["size"], curve,
        "the session's curve"
    );
    let path: Vec<Value> = line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 5)
        .into_iter()
        .enumerate()
        .map(|(i, p)| json!({ "position": p, "pressure": 0.2 * i as f32 }))
        .collect();
    let args = json!({ "brush": "Draw", "samples": path,
                       "settings": { "pressure": { "global": true } } });
    s.ok("brush.stroke", args.clone());
    let lines = stroke_lines(&log);
    let logged = &lines[0].args["settings"]["pressure"];
    assert_eq!(logged["size"], curve, "the session's curve, pinned");
    assert_eq!(
        logged["strength"],
        json!([[0.0, 0.0], [1.0, 1.0]]),
        "and the default ones"
    );
    // Another session with other global curves replays the logged line exactly.
    let mut replay = Session::new();
    replay.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    set(
        &mut replay,
        json!({ "global_pressure": { "size": [[0, 0.5], [1, 0.5]] } }),
    );
    let request = lines[0].request().unwrap_or_else(|e| panic!("{e}"));
    success(replay.request(request), "the logged stroke");
    assert_eq!(Some(replay.hash()), lines[0].hash, "replays exactly");
    std::fs::remove_file(&log).ok();
}

/// Smooth as it shipped on the morning of 2026-10-11 (decision 0036, section 1): half strength,
/// gain 32, spacing 0.025. It went back to strength 1, gain 2 and spacing 0.1 that evening.
const MORNING_SMOOTH: &str = r#"# Smooth: pulls each vertex toward the mean of its edge neighbours (boundary vertices toward
# their boundary neighbours, so open edges do not shrink in). Half strength with a high gain and
# fine spacing (decision 0036): research 21 measured that at gain 2 one full-strength stroke
# left three quarters of a test ripple, where Blender's Smooth leaves a tenth; the gain was what
# was low, so the slider now has room both ways.
name = "Smooth"
id = "brush.smooth"
family = "Smooth"
strength = 0.5
claims = ["local", "masked_unchanged", "mirror_symmetric", "spacing_invariant", "rate_invariant", "reference"]

[kernel]
type = "smooth"
mode = "uniform"
gain = 32.0

[tip]
profile = "smooth"

[stroke]
class = "dab"
spacing = 0.025
tap = 0.5
"#;

#[test]
fn a_log_recorded_before_a_retune_replays_with_the_preset_it_carries() {
    // Decision 0042. Recorded while Smooth was the morning's: the first stroke carries the
    // preset in full, the second pins its hash alone.
    let log = log_path("retuned");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let first = line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 6);
    let then = line([0.2, -0.3, 0.95], [-0.1, 0.3, 0.95], 6);
    let carried = json!({ "preset": MORNING_SMOOTH });
    s.ok("brush.stroke", stroke("Smooth", &first, &carried));
    let pinned = stroke_lines(&log)[0].args["preset_hash"].clone();
    s.ok(
        "brush.stroke",
        stroke("Smooth", &then, &json!({ "preset_hash": pinned })),
    );
    let lines = stroke_lines(&log);
    let today = s.query("brush.describe", json!({ "brush": "Smooth" }))["preset_hash"].clone();
    assert_ne!(
        pinned, today,
        "the strokes pin the morning's preset, not today's"
    );
    assert_eq!(
        lines[0].args["preset"],
        json!(MORNING_SMOOTH),
        "carried in full"
    );
    assert!(lines[1].args.get("preset").is_none(), "once per log");
    assert_eq!(lines[1].args["preset_hash"], pinned);

    // Today's build replays every line to its recorded hash, and says why.
    let mut replay = Session::new();
    replay.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    for line in &lines {
        let request = line.request().unwrap_or_else(|e| panic!("{e}"));
        let done = success(replay.request(request), "a logged stroke");
        let said = done.result["recorded_preset"].as_str().unwrap_or_default();
        assert!(
            said.contains("Smooth changed"),
            "says it used the recorded preset: {said}"
        );
        assert_eq!(
            Some(replay.hash()),
            line.hash,
            "replays to its recorded hash"
        );
    }
    // Today's Smooth on the same path ends elsewhere: the replay did use the morning's.
    let mut plain = Session::new();
    plain.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    plain.ok("brush.stroke", stroke("Smooth", &first, &json!({})));
    assert_ne!(
        Some(plain.hash()),
        lines[0].hash,
        "the retune changes the stroke"
    );
    std::fs::remove_file(&log).ok();
}

#[test]
fn a_carried_preset_must_be_the_strokes_brush_and_hash() {
    let mut s = sphere_session(None);
    let before = s.hash();
    let path = line([-0.3, 0.2, 0.95], [0.3, 0.1, 0.95], 4);
    let other = s.err(
        "brush.stroke",
        stroke("Draw", &path, &json!({ "preset": MORNING_SMOOTH })),
    );
    let wrong = json!({ "preset": MORNING_SMOOTH, "preset_hash": "0".repeat(64) });
    let unhashed = s.err("brush.stroke", stroke("Smooth", &path, &wrong));
    for err in [other, unhashed] {
        assert_eq!(code(err.code), "E_ARGS_INVALID", "{}", err.message);
        assert!(err.message.contains("preset"), "{}", err.message);
    }
    assert_eq!(s.hash(), before, "nothing changed");
}

#[test]
fn session_state_reports_the_depth_and_tilt_settings_in_effect() {
    // Decision 0053 section 8, amended 2026-10-12: each is the pen's brush's preset with this
    // session's changes on it, reported at its default too, while `brush.settings` stays the
    // changes alone.
    let mut s = Session::new();
    set(&mut s, json!({ "brush": "Clay Build" }));
    let st = state(&mut s);
    assert_eq!(st["brush.settings"], json!({}), "no change made");
    let f = |v: &Value| v.as_f64().map(|x| x as f32);
    assert_eq!(
        f(&st["brush.depth.imbed"]),
        Some(0.1),
        "the preset's own Imbed"
    );
    let defaults = [
        ("brush.depth.mask", json!(false)),
        ("brush.depth.inner", json!(-1.0)),
        ("brush.depth.outer", json!(1.0)),
        ("brush.depth.infinite", json!(false)),
        ("brush.depth.axes", json!([false, false, false])),
        ("brush.post.gravity", json!(0.0)),
        ("brush.post.gravity_dir", json!([0.0, -1.0, 0.0])),
        ("brush.tip.tilt", json!(0.0)),
        ("brush.tip.constant_tilt", json!(false)),
        ("brush.tip.align", json!(100.0)),
        ("brush.tip.alpha.adaptive", json!(1.0)),
    ];
    for (name, want) in &defaults {
        assert_eq!(&st[*name], want, "{name} at its default");
    }
    // Draw has no Imbed and no alpha to size.
    set(&mut s, json!({ "brush": "Draw" }));
    let st = state(&mut s);
    assert_eq!(st["brush.depth.imbed"], json!(0.0));
    assert_eq!(st["brush.tip.alpha.adaptive"], Value::Null, "no alpha");
    // A change shows at once, and a brush switch shows each brush's own.
    let mask = json!({ "settings": { "depth": { "mask": true, "axes": [true, false, true] } } });
    set(&mut s, mask);
    set(&mut s, json!({ "alpha": "ring" }));
    let st = state(&mut s);
    assert_eq!(st["brush.depth.mask"], json!(true));
    assert_eq!(st["brush.depth.axes"], json!([true, false, true]));
    assert_eq!(st["brush.tip.alpha.adaptive"], json!(1.0), "an alpha now");
    set(&mut s, json!({ "brush": "Clay Build" }));
    assert_eq!(
        state(&mut s)["brush.depth.mask"],
        json!(false),
        "Clay Build's"
    );
    set(&mut s, json!({ "brush": "Draw" }));
    assert_eq!(
        state(&mut s)["brush.depth.mask"],
        json!(true),
        "Draw's again"
    );
}

#[test]
fn session_state_reports_the_profile_and_curve_switches_in_effect() {
    // Decision 0059 section 4, amended 2026-10-12: the profile, Accurate Curve and Curve By
    // Pen, each the pen's brush's preset with this session's changes on it, at its default too.
    let mut s = Session::new();
    set(&mut s, json!({ "brush": "Draw" }));
    let tip = |s: &mut Session| {
        let st = state(s);
        let names = ["profile", "accurate_curve", "curve_by_pen"];
        names.map(|n| st[format!("brush.tip.{n}")].clone())
    };
    let draw = [json!("smooth"), json!(false), json!(false)];
    assert_eq!(tip(&mut s), draw, "Draw's own, no change made");
    let set_tip = json!({ "profile": "flat_top", "accurate_curve": true, "curve_by_pen": true });
    set(&mut s, json!({ "settings": { "tip": set_tip } }));
    let flat_top = [json!("flat_top"), json!(true), json!(true)];
    assert_eq!(tip(&mut s), flat_top);
    // Another brush shows its own, and Draw its own again; a custom curve reads `custom`.
    set(&mut s, json!({ "brush": "Clay" }));
    assert_eq!(tip(&mut s), draw, "Clay's own");
    set(&mut s, json!({ "brush": "Draw" }));
    assert_eq!(tip(&mut s), flat_top, "Draw's again");
    let custom = json!({ "profile": "custom", "points": [[0, 1], [1, 0]] });
    set(&mut s, json!({ "settings": { "tip": custom } }));
    assert_eq!(tip(&mut s)[0], json!("custom"));
}
