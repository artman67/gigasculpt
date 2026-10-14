//! Painting color through the engine (decision 0039): the session's colors (`color.set`), a
//! stroke with a brush that paints carrying its colors and seed so the log replays alone, and
//! Smooth blending color once a settings patch gives it a `[paint]` table.

use gs_engine::EngineConfig;
use gs_io::mesh::column;
use gs_protocol::{AttrDomain, PlainMesh, Role};
use gs_testkit::{Session, success};
use serde_json::json;

use crate::{line, log_path, sphere_session, stroke, stroke_lines};

/// The part's colors, white where it has none.
fn colors(m: &PlainMesh) -> Vec<u32> {
    let white = vec![u32::MAX; m.positions.len()];
    column(m, "color", AttrDomain::Vertex).map_or(white, <[u32]>::to_vec)
}

/// Each position as bits.
fn bits(m: &PlainMesh) -> Vec<[u32; 3]> {
    m.positions.iter().map(|p| p.map(f32::to_bits)).collect()
}

/// Green, the channel that falls as white turns red.
fn green(c: u32) -> u32 {
    c >> 8 & 255
}

#[test]
fn a_paint_stroke_pins_the_session_colors_and_its_seed_and_replays_alone() {
    let log = log_path("paint");
    let config = EngineConfig {
        log_path: Some(log.clone()),
        verify: true,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    let before = s.mesh("sphere");
    s.ok(
        "color.set",
        json!({ "main": "#ff3300", "secondary": "#102030" }),
    );
    let path = line([-0.3, 0.0, 0.96], [0.3, 0.1, 0.95], 10);
    s.ok(
        "brush.stroke",
        stroke("Paint", &path, &json!({ "size": 0.3 })),
    );
    s.ok(
        "brush.stroke",
        stroke("Paint", &path, &json!({ "invert": true, "size": 0.15 })),
    );
    let after = s.mesh("sphere");
    assert_eq!(bits(&after), bits(&before), "Paint moves no point");
    let painted = colors(&after);
    let main = painted
        .iter()
        .filter(|&&c| c & 0xff == 255 && green(c) < 128)
        .count();
    let secondary = painted.iter().filter(|&&c| c & 0xff < 64).count();
    assert!(
        main > 10 && secondary > 10,
        "{main} reddened, {secondary} darkened"
    );
    let lines = stroke_lines(&log);
    for line in &lines {
        assert_eq!(
            line.args["colors"],
            json!({ "main": "#ff3300", "secondary": "#102030" }),
            "the log keeps the session's colors"
        );
        assert!(line.args["seed"].is_u64(), "and the seed: {}", line.args);
    }
    // A fresh session with other colors replays both strokes alone.
    let mut replay = sphere_session(None);
    replay.ok("color.set", json!({ "main": "#0000ff" }));
    for line in &lines {
        let request = line.request().unwrap_or_else(|e| panic!("{e}"));
        success(replay.request(request), "the logged stroke");
        assert_eq!(Some(replay.hash()), line.hash, "replays exactly");
    }
    std::fs::remove_file(&log).ok();
}

#[test]
fn smooth_blends_color_with_a_paint_patch_and_leaves_it_without() {
    // A small red spot on the sphere's front, then Smooth across it with and without `[paint]`.
    let mut s = sphere_session(None);
    let spot = json!({ "size": 0.06, "colors": { "main": "#ff0000", "secondary": "#000000" } });
    s.ok(
        "brush.stroke",
        stroke(
            "Paint",
            &line([-0.05, 0.0, 1.0], [0.05, 0.0, 1.0], 4),
            &spot,
        ),
    );
    let spotted = colors(&s.mesh("sphere"));
    let across = line([-0.25, 0.0, 0.97], [0.25, 0.0, 0.97], 8);
    s.ok(
        "brush.stroke",
        stroke("Smooth", &across, &json!({ "size": 0.2 })),
    );
    assert_eq!(
        colors(&s.mesh("sphere")),
        spotted,
        "Smooth alone leaves color"
    );
    s.ok("history.undo", json!({}));
    let patch = json!({ "size": 0.2, "settings": { "paint": { "channel": "color" } } });
    s.ok("brush.stroke", stroke("Smooth", &across, &patch));
    let blended = colors(&s.mesh("sphere"));
    let pink = |c: &[u32]| c.iter().filter(|&&c| c != u32::MAX).count();
    let reddest = |c: &[u32]| c.iter().map(|&c| green(c)).min().unwrap_or(255);
    assert!(
        blended.iter().all(|&c| c & 0xff == 255),
        "red stays full where white turns pink"
    );
    assert!(
        pink(&blended) > pink(&spotted) && reddest(&blended) > reddest(&spotted),
        "the spot spreads ({} to {} points) and pales (green {} to {})",
        pink(&spotted),
        pink(&blended),
        reddest(&spotted),
        reddest(&blended)
    );
}

#[test]
fn color_set_keeps_the_session_colors_and_refuses_what_is_not_a_color() {
    let mut s = Session::new();
    let now = s.query("color.set", json!({}));
    assert_eq!(now, json!({ "main": "#ffffff", "secondary": "#000000" }));
    let now = s.query("color.set", json!({ "main": "#FF8000", "gradient": true }));
    let want = json!({ "main": "#ff8000", "secondary": "#000000", "gradient": true });
    assert_eq!(now, want, "a color left out is kept");
    for (args, path) in [
        (json!({ "main": "red" }), "/main"),
        (json!({ "secondary": "#12345" }), "/secondary"),
        (json!({ "main": "#12345g" }), "/main"),
    ] {
        let err = s.err("color.set", args.clone());
        assert_eq!(err.path.as_deref(), Some(path), "{args}: {}", err.message);
    }
    assert_eq!(
        s.query("color.set", json!({})),
        want,
        "refused changes change nothing"
    );
}

#[test]
fn a_brush_that_paints_no_color_takes_no_colors_or_seed() {
    let mut s = sphere_session(None);
    let path = line([-0.3, 0.0, 1.0], [0.3, 0.0, 1.0], 8);
    let colors = json!({ "colors": { "main": "#ff0000", "secondary": "#000000" } });
    for (extra, at) in [(colors, "/colors"), (json!({ "seed": 7 }), "/seed")] {
        let err = s.err("brush.stroke", stroke("Draw", &path, &extra));
        assert_eq!(err.path.as_deref(), Some(at), "{extra}: {}", err.message);
    }
}
