//! A stroke is one undo step, however many samples it has; the command log replays strokes bit
//! for bit; the thread count never changes a stroke; and a stroke inside a batch starts from
//! where the batch's earlier steps left the part.

use gs_engine::EngineConfig;
use gs_protocol::{Role, ScriptLine};
use gs_testkit::{Session, success};
use serde_json::{Value, json};

use crate::{line, log_path, sphere_session, stroke};

/// Strokes with several brushes, classes and options.
fn strokes() -> Vec<Value> {
    let across = line([-0.4, 0.1, 0.92], [0.4, 0.2, 0.9], 12);
    let mirror = json!({ "symmetry": { "mirror": { "x": true } }, "size": "8%" });
    let radial = json!({ "symmetry": { "radial": { "axis": "z", "count": 6 } }, "size": 0.15 });
    let colors = json!({ "main": "#ff3300", "secondary": "#102030" });
    vec![
        stroke("Draw", &across, &json!({ "modifier": 40 })),
        stroke("Clay Build", &across, &mirror),
        // Live Detail, so the strokes after it sculpt a mesh whose topology changed.
        stroke("Draw", &across, &json!({ "live_detail": { "size": 0.15 } })),
        // Clay's alpha swapped for a tiled, blurred ring that raises and carves.
        stroke(
            "Clay",
            &across,
            &json!({ "settings": { "tip": { "alpha": {
                "image": "ring", "mid": 0.4, "blur": 2, "tiles": [2, 2] } } } }),
        ),
        stroke(
            "brush.smooth",
            &line([0.0, 0.3, 0.95], [0.2, 0.0, 0.98], 8),
            &radial,
        ),
        stroke(
            "Move",
            &line([0.0, -0.2, 0.98], [0.1, -0.3, 1.3], 6),
            &json!({}),
        ),
        stroke(
            "Pull Out",
            &line([0.5, 0.0, 0.86], [0.8, 0.1, 1.2], 9),
            &json!({}),
        ),
        stroke("Mask Pen", &across, &json!({ "invert": false })),
        // Painting: Main, Secondary with Alt, then blending it, alone and while smoothing.
        stroke("Paint", &across, &json!({ "colors": colors })),
        stroke("Paint", &across, &json!({ "invert": true, "size": 0.1 })),
        stroke("Paint Blend", &across, &json!({})),
        stroke(
            "Smooth",
            &across,
            &json!({ "settings": { "paint": { "channel": "color" } } }),
        ),
        stroke("Flat Polish", &across, &json!({ "lock": { "x": true } })),
        stroke(
            "Clip Curve",
            &line([-1.2, 0.6, 1.0], [1.2, 0.5, 1.0], 4),
            &json!({}),
        ),
        on_screen(
            "Draw",
            "front",
            &[[420.0, 500.0], [520.0, 470.0], [610.0, 520.0]],
        ),
        on_screen("Move", "iso", &[[500.0, 500.0], [560.0, 430.0]]),
        on_screen("Clip Curve", "front", &[[100.0, 300.0], [900.0, 380.0]]),
        on_screen(
            "Clay Line",
            "right",
            &[[400.0, 600.0], [450.0, 560.0], [640.0, 520.0]],
        ),
        on_screen(
            "Pull Out",
            "top",
            &[[520.0, 520.0], [560.0, 540.0], [600.0, 580.0]],
        ),
    ]
}

/// A stroke of screen samples through a named view.
fn on_screen(brush: &str, view: &str, points: &[[f32; 2]]) -> Value {
    let samples: Vec<Value> = (points.iter())
        .map(|p| json!({ "position": p, "pressure": 0.7 }))
        .collect();
    json!({ "brush": brush, "camera": view, "size_px": 48, "samples": samples })
}

/// Runs every stroke; the scene hash after each.
fn run_strokes(s: &mut Session) -> Vec<String> {
    (strokes().into_iter())
        .map(|args| {
            s.ok("brush.stroke", args);
            s.hash()
        })
        .collect()
}

#[test]
fn every_stroke_is_one_undo_step() {
    let mut s = sphere_session(None);
    let start = s.hash();
    let mut hashes = Vec::new();
    for (k, args) in strokes().into_iter().enumerate() {
        let done = s.ok("brush.stroke", args);
        assert!(done.history_id.is_some(), "stroke {k} made a step");
        hashes.push(s.hash());
    }
    let steps = s.history()["steps"].as_array().map_or(0, Vec::len);
    assert_eq!(
        steps,
        hashes.len() + 1,
        "one step per stroke, after the sphere's"
    );
    for want in hashes.iter().rev().skip(1).chain([&start]) {
        s.ok("history.undo", json!({}));
        assert_eq!(&s.hash(), want, "one undo takes back one whole stroke");
    }
    for want in &hashes {
        s.ok("history.redo", json!({}));
        assert_eq!(&s.hash(), want, "and redo puts it back");
    }
}

#[test]
fn the_log_replays_every_stroke_bit_for_bit() {
    let log = log_path("strokes");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let hashes = run_strokes(&mut s);
    let text = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("no log: {e}"));
    let mut replay = Session::with(EngineConfig::default(), Role::Agent);
    let mut lines = 0;
    for line in text.lines() {
        let Ok(ScriptLine::Record(record)) = ScriptLine::parse(line) else {
            continue;
        };
        let request = record.request().unwrap_or_else(|e| panic!("{e}"));
        success(replay.request(request), &record.command);
        assert_eq!(
            Some(replay.hash()),
            record.hash,
            "{} replays exactly",
            record.command
        );
        lines += 1;
    }
    assert_eq!(lines, hashes.len() + 1, "one log line per call");
    assert_eq!(
        replay.hash(),
        hashes[hashes.len() - 1],
        "the replay ends where the session did"
    );
    std::fs::remove_file(&log).ok();
}

#[test]
fn strokes_give_the_same_mesh_at_any_thread_count() {
    let run = |threads| {
        gs_par::with_threads(threads, || {
            let mut s = sphere_session(None);
            run_strokes(&mut s)
        })
    };
    assert_eq!(run(1), run(24), "1 and 24 threads give the same hashes");
}

#[test]
fn a_stroke_in_a_batch_starts_where_the_batch_left_the_part() {
    let shift = json!({ "direction": [1, 0, 0], "distance": 0.1 });
    let mut moved = sphere_session(None);
    moved.ok("mesh.translate", shift.clone());
    // Grab the shifted sphere's front, drag it out, and back to the grab.
    let grab = [0.1, 0.0, 1.0];
    let there_and_back = [grab, [0.1, 0.3, 1.3], grab];
    let mut s = sphere_session(None);
    let start = s.hash();
    let drag = stroke("Move", &there_and_back, &json!({}));
    let done = success(
        s.batch(&[("mesh.translate", shift), ("brush.stroke", drag)]),
        "batch",
    );
    assert_eq!(
        s.hash(),
        moved.hash(),
        "dragging back keeps the shift before it"
    );
    assert!(done.history_id.is_some(), "the batch is a step");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), start, "one undo takes back the whole batch");
}
