//! Primitives made with detail levels (decision 0047, the F26b amendment): the base at the
//! primitive's own density with `levels` made above it by `level.subdivide`'s code, the sphere
//! round and even at its top and exactly its base at the lowest level, the cube's edges and the
//! cylinder's caps kept, one undo step, the same bits on any thread count and on replay, and
//! calls without `levels` logged and made exactly as before.

use std::collections::BTreeSet;
use std::path::PathBuf;

use gs_protocol::{ErrorCode, NO_VERTEX, PlainMesh};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::determinism::seeded;
use crate::socket::scratch;
use crate::support::{EngineSession, agent, log_records, stroke};

/// The scene `old_calls` make on a session with seed 3, as the engine at `61e4d2f6` (before
/// `levels`) hashed it.
const OLD_HASH: &str = "8703710e50114e57a805806a211847cb8aabc76aca6405e2c84354896aeedcdd";

/// Calls as logs written before `levels` hold them, give or take their pinned defaults.
fn old_calls() -> [(&'static str, Value); 5] {
    [
        ("primitive.sphere", json!({})),
        ("primitive.sphere", json!({ "segments": 8, "axis": "z" })),
        ("primitive.cube", json!({ "size": 1.5, "axis": "x" })),
        ("primitive.cylinder", json!({})),
        ("part.add", json!({ "shape": "sphere" })),
    ]
}

/// Faces of each level of `part`, the base first.
fn faces(s: &mut Session, part: &str) -> Vec<u64> {
    let described = s.query("level.describe", json!({ "part": part }));
    let levels = (described["levels"].as_array()).unwrap_or_else(|| panic!("{described}"));
    (levels.iter())
        .map(|l| l["faces"].as_u64().unwrap_or_else(|| panic!("{l}")))
        .collect()
}

/// The corners of a face, without the triangle marker.
fn corners(face: &[u32; 4]) -> impl Iterator<Item = u32> + '_ {
    face.iter().copied().filter(|&v| v != NO_VERTEX)
}

/// The longest edge of `mesh` over its shortest.
fn edge_ratio(mesh: &PlainMesh) -> f64 {
    let (mut short, mut long) = (f64::MAX, 0.0f64);
    for face in &mesh.faces {
        let c: Vec<u32> = corners(face).collect();
        for (i, &a) in c.iter().enumerate() {
            let b = c[(i + 1) % c.len()];
            let d = distance(mesh.positions[a as usize], mesh.positions[b as usize]);
            (short, long) = (short.min(d), long.max(d));
        }
    }
    long / short
}

/// The distance between two points, in f64.
fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    (0..3)
        .map(|i| f64::from(a[i]) - f64::from(b[i]))
        .map(|d| d * d)
        .sum::<f64>()
        .sqrt()
}

/// The points' bits, sorted: two meshes numbered differently have the same points when these
/// match.
fn point_set(mesh: &PlainMesh) -> Vec<[u32; 3]> {
    let mut points: Vec<[u32; 3]> = (mesh.positions.iter())
        .map(|p| p.map(f32::to_bits))
        .collect();
    points.sort_unstable();
    points
}

/// The faces as sets of their corners' bits: two meshes numbered differently have the same
/// faces when these match.
fn face_set(mesh: &PlainMesh) -> BTreeSet<Vec<[u32; 3]>> {
    (mesh.faces.iter())
        .map(|face| {
            let mut c: Vec<[u32; 3]> = corners(face)
                .map(|v| mesh.positions[v as usize].map(f32::to_bits))
                .collect();
            c.sort_unstable();
            c
        })
        .collect()
}

#[test]
fn a_sphere_with_levels_steps_down_to_exactly_its_base() {
    let mut s = agent();
    let made = s.ok(
        "primitive.sphere",
        json!({ "subdivisions": 4, "levels": 4 }),
    );
    assert_eq!(made.result["faces"], json!(24_576), "the top is active");
    assert_eq!(faces(&mut s, "sphere"), [96, 384, 1_536, 6_144, 24_576]);
    let described = s.query("level.describe", json!({ "part": "sphere" }));
    assert_eq!(described["active"], json!(5), "{described}");
    assert!((described["levels"].as_array().unwrap().iter()).all(|l| l["smooth"] == json!(true)));
    s.ok("level.set", json!({ "part": "sphere", "level": 1 }));
    s.ok(
        "primitive.sphere",
        json!({ "name": "low", "subdivisions": 4 }),
    );
    let (lowest, low) = (s.mesh("sphere"), s.mesh("low"));
    assert_eq!(point_set(&lowest), point_set(&low), "the base's points");
    assert_eq!(face_set(&lowest), face_set(&low), "the base's faces");
}

/// Every point of the New button's sphere's top is on the sphere, and its edges are within a
/// stated ratio of each other. Today's single-mesh sphere at the same face count
/// (`subdivisions` 64, ratio 2.10) is measured beside it.
#[test]
fn the_top_of_a_sphere_with_levels_is_round_and_even() {
    let mut s = agent();
    let center = [1.0f32, -2.0, 0.5];
    let args = json!({ "subdivisions": 2, "levels": 5, "radius": 2, "center": center });
    s.ok("primitive.sphere", args);
    let top = s.mesh("sphere");
    for p in &top.positions {
        let off = (distance(*p, center) - 2.0).abs();
        assert!(off < 4e-6, "{p:?} is {off} off the sphere");
    }
    s.ok(
        "primitive.sphere",
        json!({ "name": "today", "subdivisions": 64 }),
    );
    let (ratio, today) = (edge_ratio(&top), edge_ratio(&s.mesh("today")));
    assert!(
        ratio < EVEN,
        "edge ratio {ratio} (today's single mesh: {today})"
    );
}

/// The longest edge of the New button's sphere's top over its shortest stays under this
/// (measured 3.33 on main by `level.subdivide` and a projection).
const EVEN: f64 = 3.4;

#[test]
fn a_stroke_on_the_lowest_level_carries_up_to_the_top() {
    let mut s = agent();
    s.ok(
        "primitive.sphere",
        json!({ "subdivisions": 4, "levels": 4 }),
    );
    let top = s.mesh("sphere");
    s.ok("level.set", json!({ "level": 1 }));
    s.ok("brush.stroke", stroke([0.0, 0.0, 1.0]));
    let up = s.ok("level.set", json!({ "level": 5 }));
    assert!(
        up.result["carried"].as_u64().is_some_and(|n| n > 0),
        "{up:?}"
    );
    let moved = s.mesh("sphere");
    let changed = |p: &[f32; 3], q: &[f32; 3]| p.map(f32::to_bits) != q.map(f32::to_bits);
    let near = (top.positions.iter().zip(&moved.positions))
        .filter(|(p, q)| changed(p, q))
        .map(|(p, _)| p[2])
        .fold(f32::MAX, f32::min);
    assert!(near > 0.0, "only the front moved, from z = {near}");
    let count = (top.positions.iter().zip(&moved.positions))
        .filter(|(p, q)| changed(p, q))
        .count();
    assert!(count > 100, "{count} top points moved");
}

/// Every level of a cube with levels is the cube with that level's grid, point for point: its
/// sides stay flat and its edges sharp.
#[test]
fn a_cube_with_levels_is_the_denser_cube_at_every_level() {
    let mut s = agent();
    s.ok("primitive.cube", json!({ "levels": 3 }));
    assert_eq!(faces(&mut s, "cube"), [6, 24, 96, 384]);
    for (level, n) in [(4, 8), (3, 4), (2, 2), (1, 1)] {
        s.ok("level.set", json!({ "part": "cube", "level": level }));
        let grid = format!("grid-{n}");
        s.ok("primitive.cube", json!({ "name": grid, "subdivisions": n }));
        let (got, want) = (s.mesh("cube"), s.mesh(&grid));
        assert_eq!(point_set(&got), point_set(&want), "level {level}");
        assert_eq!(face_set(&got), face_set(&want), "level {level}");
    }
}

/// The cylinder's caps stay flat at its two ends and its side round at its radius.
#[test]
fn a_cylinder_with_levels_keeps_flat_caps_and_a_round_side() {
    let mut s = agent();
    let args = json!({
        "segments": 12, "divisions": 4, "cap_rings": 2, "levels": 3,
        "radius": 0.5, "height": 3, "axis": "z", "center": [1, 2, 3]
    });
    s.ok("primitive.cylinder", args);
    assert_eq!(faces(&mut s, "cylinder"), [96, 360, 1_440, 5_760]);
    let top = s.mesh("cylinder");
    let across = |p: &[f32; 3]| libm::hypot(f64::from(p[0]) - 1.0, f64::from(p[1]) - 2.0);
    let (mut caps, mut widest) = (0, 0.0f64);
    for p in &top.positions {
        assert!((1.5..=4.5).contains(&p[2]), "{p:?} is past a cap");
        widest = widest.max(across(p));
        if [1.5f32, 4.5].map(f32::to_bits).contains(&p[2].to_bits()) {
            caps += 1;
        } else {
            let off = (across(p) - 0.5).abs() / 0.5;
            assert!(off < ROUND, "{p:?} is {off} of the radius off the side");
        }
    }
    assert!(
        (widest - 0.5).abs() < 1e-6,
        "the side's widest point {widest}"
    );
    // Each cap is a disc of 1,344 quads with 96 rim edges: 1 + (4 * 1,344 + 96) / 2 - 1,344.
    assert_eq!(caps, 2 * 1_393, "points on the caps");
}

/// How far, as a share of the radius, a 12-segment cylinder's side may stray from round after
/// three levels (measured 2.3e-4 on main: the rim's cubic B-spline ripple).
const ROUND: f64 = 3e-4;

/// The box's three sizes, the uv form's levels, and a sphere's radius and center at the top
/// land where their arguments say; levels past the range, or a top too big to build, are
/// refused.
#[test]
fn the_new_arguments_land_where_they_say() {
    let mut s = agent();
    for axis in ["x", "y", "z"] {
        let name = format!("box-{axis}");
        let args = json!({ "name": name, "size": [3, 2, 1], "levels": 2, "axis": axis });
        s.ok("primitive.cube", args);
        let mesh = s.mesh(&name);
        for (i, half) in [1.5f32, 1.0, 0.5].into_iter().enumerate() {
            let along: BTreeSet<u32> = mesh.positions.iter().map(|p| p[i].to_bits()).collect();
            let ends = [(-half).to_bits(), half.to_bits()];
            assert!(
                along.contains(&ends[0]) && along.contains(&ends[1]),
                "{axis}, {i}"
            );
            assert!(
                (mesh.positions.iter()).all(|p| p[i].abs() <= half),
                "{axis}, {i}"
            );
        }
    }
    s.ok(
        "primitive.sphere",
        json!({ "name": "globe", "segments": 8, "rings": 4, "levels": 2, "axis": "x" }),
    );
    assert_eq!(faces(&mut s, "globe"), [32, 112, 448]);
    let err = s.err("primitive.sphere", json!({ "levels": 9 }));
    assert_eq!(err.code, ErrorCode::ArgsInvalid, "{err:?}");
    let err = s.err(
        "primitive.sphere",
        json!({ "subdivisions": 256, "levels": 8 }),
    );
    assert_eq!(err.code, ErrorCode::Limit, "{err:?}");
    assert!(err.message.contains("faces"), "{err:?}");
}

#[test]
fn making_a_primitive_with_levels_is_one_undo_step() {
    let mut s = agent();
    let before = (s.hash(), s.history()["undoable"].as_u64());
    s.ok("primitive.cylinder", json!({ "segments": 12, "levels": 2 }));
    let made = s.hash();
    let undoable = s.history()["undoable"].as_u64();
    assert_eq!(undoable, before.1.map(|n| n + 1), "one step");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before.0, "undo takes the whole make back");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), made, "redo brings back the stack");
    let described = s.query("level.describe", json!({ "part": "cylinder" }));
    assert_eq!(
        (described["count"].clone(), described["active"].clone()),
        (json!(3), json!(3))
    );
}

/// Calls that make primitives with levels, of every primitive and form.
fn leveled_calls() -> [(&'static str, Value); 4] {
    [
        (
            "primitive.sphere",
            json!({ "subdivisions": 3, "levels": 3 }),
        ),
        (
            "primitive.sphere",
            json!({ "segments": 8, "rings": 6, "levels": 2, "axis": "z" }),
        ),
        (
            "primitive.cube",
            json!({ "size": [2, 1, 1], "levels": 3, "axis": "x" }),
        ),
        (
            "primitive.cylinder",
            json!({ "segments": 12, "divisions": 4, "cap_rings": 2, "levels": 2 }),
        ),
    ]
}

#[test]
fn primitives_with_levels_give_the_same_bits_on_any_thread_count_and_on_replay() {
    let dir = scratch("threads");
    let log = dir.join("leveled.jsonl");
    let script = |log: Option<PathBuf>| {
        let mut s = seeded(3, log);
        for (command, args) in leveled_calls() {
            s.ok(command, args);
        }
        s.hash()
    };
    let one = gs_par::with_threads(1, || script(None));
    let many = gs_par::with_threads(24, || script(Some(log.clone())));
    assert_eq!(one, many, "the same bits on 1 and 24 threads");
    let (_, records) = log_records(&log);
    let made = (records.iter()).filter(|r| r.command.starts_with("primitive."));
    let logged: Vec<Option<u64>> = made.map(|r| r.args["levels"].as_u64()).collect();
    assert_eq!(
        logged,
        [Some(3), Some(2), Some(3), Some(2)],
        "pinned levels"
    );
    let mut replay = seeded(4, None);
    EngineSession::replay_log(&mut replay, &log);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(replay.hash(), many, "the log replays bit for bit");
}

/// A call with `levels` 0 is logged without it and makes what the call without it makes, and
/// calls shaped like old log lines make exactly the scene they made before `levels` existed.
#[test]
fn calls_without_levels_log_and_make_what_they_did_before() {
    let dir = scratch("old");
    let log = dir.join("zero.jsonl");
    let mut zero = seeded(3, Some(log.clone()));
    for (command, mut args) in old_calls() {
        if command != "part.add" {
            args["levels"] = json!(0);
        }
        zero.ok(command, args);
    }
    let (_, records) = log_records(&log);
    std::fs::remove_dir_all(&dir).ok();
    let made: Vec<&Value> = (records.iter())
        .filter(|r| r.command.starts_with("primitive."))
        .map(|r| &r.args)
        .collect();
    assert_eq!(made.len(), 4, "{made:?}");
    assert!(
        made.iter().all(|args| args.get("levels").is_none()),
        "{made:?}"
    );
    let mut old = seeded(3, None);
    for (command, args) in old_calls() {
        old.ok(command, args);
    }
    assert_eq!(
        old.hash(),
        OLD_HASH,
        "the scene the engine made before levels"
    );
    assert_eq!(zero.hash(), OLD_HASH, "levels 0 is no levels");
}
