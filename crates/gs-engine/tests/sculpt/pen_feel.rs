//! Pressure and the pen's feel through the commands (catalogs `brush-settings.tablet-pressure`
//! and `interface.pen-settings`, decision 0036 section 3 as amended): a constant pressure
//! strokes as its curve gives, no pressure is full pressure, and `pen.set`'s sensitivities and
//! lazy pressure reach every stroke, pinned so its log line replays alone.

use gs_engine::EngineConfig;
use gs_io::mesh::column;
use gs_protocol::{AttrDomain, ErrorCode, PlainMesh, Role};
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::{line, log_path, sphere_session, stroke_lines, with};

/// The path every stroke here takes, across the front of the unit sphere.
fn path() -> Vec<[f32; 3]> {
    line([-0.3, 0.0, 0.96], [0.3, 0.1, 0.95], 12)
}

/// `brush.stroke` arguments for `brush` along the path, sample `i` at `pressures[i]` (cycled;
/// none given, none sent), with `extra` arguments.
fn pressed(brush: &str, pressures: &[f32], extra: &Value) -> Value {
    let samples: Vec<Value> = (path().iter().enumerate())
        .map(|(i, p)| match pressures.get(i % pressures.len().max(1)) {
            Some(pr) => json!({ "position": p, "pressure": pr }),
            None => json!({ "position": p }),
        })
        .collect();
    with(json!({ "brush": brush, "samples": samples }), extra)
}

/// A fresh sphere after `setup` calls and one stroke with `args`.
fn after(setup: &[(&str, Value)], args: Value) -> Session {
    let mut s = sphere_session(None);
    for (command, a) in setup {
        s.ok(command, a.clone());
    }
    s.ok("brush.stroke", args);
    s
}

/// The farthest a moved vertex lies from the path's middle after one dab of Draw at
/// `pressure`, the session's feel and curves as they are. The dab is undone after.
fn reach(s: &mut Session, pressure: f32) -> f32 {
    let at = path()[6];
    let before = s.mesh("sphere");
    let sample = json!({ "position": at, "pressure": pressure });
    let args = json!({ "brush": "Draw", "size": 0.3, "samples": [sample] });
    s.ok("brush.stroke", args);
    let after = s.mesh("sphere");
    s.ok("history.undo", json!({}));
    (before.positions.iter().zip(&after.positions))
        .filter(|(a, b)| a.map(f32::to_bits) != b.map(f32::to_bits))
        .map(|(a, _)| {
            (0..3)
                .map(|k| (a[k] - at[k]) * (a[k] - at[k]))
                .sum::<f32>()
                .sqrt()
        })
        .fold(0.0, f32::max)
}

/// The part's colors, white where it has none.
fn colors(m: &PlainMesh) -> Vec<u32> {
    let white = vec![u32::MAX; m.positions.len()];
    column(m, "color", AttrDomain::Vertex).map_or(white, <[u32]>::to_vec)
}

#[test]
fn a_constant_pressure_strokes_as_its_curve_gives_and_none_is_full() {
    // Strength 0.5 at pressure 0.5 through a curve that gives 0.8 there is strength 0.4.
    let curve = json!({ "pressure": { "strength": [[0, 0], [0.5, 0.8], [1, 1]] } });
    let at = |strength: f32, pressures: &[f32]| {
        let extra = json!({ "size": 0.3, "strength": strength, "settings": curve });
        after(&[], pressed("Draw", pressures, &extra)).hash()
    };
    let half = at(0.5, &[0.5]);
    let full = at(0.4, &[1.0]);
    assert_ne!(full, sphere_session(None).hash(), "it sculpts");
    assert_eq!(half, full, "the curve's value at 0.5 scales the strength");
    // A mouse: no pressure on any sample is pressure 1.
    assert_eq!(at(0.4, &[]), full, "no pressure presses fully");
}

#[test]
fn size_sensitivity_scales_the_size_curve_and_turns_it_round() {
    let mut s = sphere_session(None);
    // No size curve: the sensitivity has nothing to scale.
    s.ok("pen.set", json!({ "size_sensitivity": -50 }));
    let (light, firm) = (reach(&mut s, 0.2), reach(&mut s, 1.0));
    assert!(
        firm > 0.25 && (light - firm).abs() < 0.02,
        "no size curve, the size stays: {light} and {firm}"
    );
    // The straight size curve at 100: a firmer touch grows the dab.
    s.ok("pen.set", json!({ "size_sensitivity": 100 }));
    s.ok(
        "brush.set",
        json!({ "global_pressure": { "size": [[0, 0], [1, 1]] } }),
    );
    let (light, firm) = (reach(&mut s, 0.2), reach(&mut s, 1.0));
    assert!(light < 0.5 * firm, "it grows: {light} against {firm}");
    // At -50 it runs backwards and half as far: 1 - 0.5 p, so it shrinks to half.
    s.ok("pen.set", json!({ "size_sensitivity": -50 }));
    let (light, firm) = (reach(&mut s, 0.2), reach(&mut s, 1.0));
    assert!(
        firm < 0.75 * light && firm > 0.4 * light,
        "it shrinks as pressure rises: {light} against {firm}"
    );
    let state = s.query("brush.set", json!({}));
    assert_eq!(
        state["size_curve"],
        json!([[0.0, 1.0], [1.0, 0.5]]),
        "the size curve in effect, as the ring reads it: {state}"
    );
}

#[test]
fn strength_and_color_sensitivity_at_0_leave_pressure_out() {
    let extra = json!({ "size": 0.3 });
    let draw = |setup: &[(&str, Value)], p: f32| after(setup, pressed("Draw", &[p], &extra));
    let paint = |setup: &[(&str, Value)], p: f32| {
        colors(&after(setup, pressed("Paint", &[p], &extra)).mesh("sphere"))
    };
    let red = ("color.set", json!({ "main": "#ff3300" }));
    // As shipped, pressure changes both.
    assert_ne!(draw(&[], 0.2).hash(), draw(&[], 1.0).hash());
    let alone = std::slice::from_ref(&red);
    assert_ne!(paint(alone, 0.2), paint(alone, 1.0));
    // Strength Sensitivity 0: the sculpt is the same at any pressure.
    let flat = ("pen.set", json!({ "strength_sensitivity": 0 }));
    assert_eq!(
        draw(std::slice::from_ref(&flat), 0.2).hash(),
        draw(&[flat], 1.0).hash(),
        "Strength Sensitivity 0 leaves pressure out"
    );
    // Color Sensitivity 0: the paint is the same color at any pressure, the sculpt is not.
    let flat = ("pen.set", json!({ "color_sensitivity": 0 }));
    let (light, firm) = (
        paint(&[red.clone(), flat.clone()], 0.2),
        paint(&[red, flat.clone()], 1.0),
    );
    assert!(light.iter().any(|&c| c != u32::MAX), "it paints");
    assert_eq!(light, firm, "equal color at 0.2 and 1.0");
    assert_ne!(
        draw(std::slice::from_ref(&flat), 0.2).hash(),
        draw(&[flat], 1.0).hash(),
        "the sculpt still follows the pen"
    );
}

#[test]
fn lazy_pressure_moves_part_of_the_way_toward_each_sample() {
    // A jittery pen: each sample far from the last.
    let raw = [
        0.3, 0.9, 0.35, 0.85, 0.3, 0.95, 0.4, 0.8, 0.3, 0.9, 0.35, 0.85,
    ];
    let lazy = 0.5_f32;
    let mut eased: Vec<f32> = Vec::new();
    for &p in &raw {
        let next = eased.last().map_or(p, |&q| q + (1.0 - lazy) * (p - q));
        eased.push(next);
    }
    let swing = |ps: &[f32]| {
        (ps.windows(2))
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    };
    assert!(swing(&eased) < 0.5 * swing(&raw), "less jitter: {eased:?}");
    let curves = json!({ "global_pressure": { "size": [[0, 0.25], [1, 1]] } });
    let set = ("brush.set", curves);
    let extra = json!({ "size": 0.3 });
    let mut felt = after(
        &[set.clone(), ("pen.set", json!({ "lazy_pressure": lazy }))],
        pressed("Draw", &raw, &extra),
    );
    let mut by_hand = after(std::slice::from_ref(&set), pressed("Draw", &eased, &extra));
    let mut plain = after(&[set], pressed("Draw", &raw, &extra));
    assert_ne!(
        felt.hash(),
        plain.hash(),
        "lazy pressure changes the stroke"
    );
    assert_eq!(
        felt.hash(),
        by_hand.hash(),
        "exactly as the eased pressures"
    );
}

#[test]
fn the_feel_is_pinned_in_the_stroke_and_only_when_it_is_not_the_default() {
    let log = log_path("pen-feel");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let extra = json!({ "size": 0.3 });
    let ramp = [0.2, 0.4, 0.6, 0.8, 1.0];
    s.ok("brush.stroke", pressed("Draw", &ramp, &extra));
    let feel = json!({ "size_sensitivity": -50, "strength_sensitivity": 30, "lazy_pressure": 0.5 });
    let answer = s.query("pen.set", feel);
    assert_eq!(answer["color_sensitivity"], json!(100.0), "{answer}");
    s.ok(
        "brush.set",
        json!({ "global_pressure": { "size": [[0, 0], [1, 1]] } }),
    );
    s.ok("brush.stroke", pressed("Draw", &ramp, &extra));
    // A stroke that gives its own feel keeps it.
    let own = with(
        pressed("Draw", &ramp, &extra),
        &json!({ "pen": { "color_sensitivity": 10 } }),
    );
    s.ok("brush.stroke", own);
    s.ok(
        "pen.set",
        json!({ "size_sensitivity": 100, "strength_sensitivity": 100, "lazy_pressure": 0 }),
    );
    s.ok("brush.stroke", pressed("Draw", &ramp, &extra));
    let lines = stroke_lines(&log);
    assert_eq!(lines.len(), 4);
    assert!(lines[0].args.get("pen").is_none(), "{}", lines[0].args);
    let pinned = &lines[1].args["pen"];
    let want = json!({
        "size_sensitivity": -50.0,
        "strength_sensitivity": 30.0,
        "color_sensitivity": 100.0,
        "lazy_pressure": 0.5
    });
    assert_eq!(pinned, &want, "{}", lines[1].args);
    assert_eq!(lines[2].args["pen"]["color_sensitivity"], json!(10.0));
    assert_eq!(lines[2].args["pen"]["size_sensitivity"], json!(100.0));
    assert!(lines[3].args.get("pen").is_none(), "{}", lines[3].args);
    // Each line replays alone in a session with the default feel and no curves.
    let mut replay = sphere_session(None);
    for line in &lines {
        let request = line.request().unwrap_or_else(|e| panic!("{e}"));
        success(replay.request(request), "the logged stroke");
        assert_eq!(Some(replay.hash()), line.hash, "replays exactly");
    }
}

#[test]
fn pen_set_refuses_what_is_out_of_range_and_reports_its_feel() {
    let mut s = sphere_session(None);
    for bad in [
        json!({ "size_sensitivity": 101 }),
        json!({ "color_sensitivity": -100.5 }),
        json!({ "lazy_pressure": 1.5 }),
        json!({ "lazy_pressure": -0.1 }),
    ] {
        let e = s.err("pen.set", bad.clone());
        assert_eq!(e.code, ErrorCode::ArgsInvalid, "{bad}: {}", e.message);
    }
    let state = s.query("session.state", json!({}));
    assert_eq!(state["pen.size_sensitivity"], json!(100.0));
    assert_eq!(state["pen.lazy_pressure"], json!(0.0));
    s.ok(
        "pen.set",
        json!({ "strength_sensitivity": -20, "lazy_pressure": 0.3 }),
    );
    let state = s.query("session.state", json!({}));
    assert_eq!(state["pen.strength_sensitivity"], json!(-20.0));
    assert_eq!(state["pen.color_sensitivity"], json!(100.0));
    let lazy = state["pen.lazy_pressure"].as_f64().unwrap_or_default();
    assert!((lazy - 0.3).abs() < 1e-6, "{lazy}");
}
