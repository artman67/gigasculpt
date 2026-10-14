//! `mesh.diff`: two meshes with the same topology compared vertex by vertex, as parts now or in
//! a checkpoint, and meshes of different topology reported rather than refused.
//!
//! Arguments: `a` and `b`, each `part:NAME` or `checkpoint:NAME/PART`, and `epsilon`. The result
//! holds `a` and `b` (`{ "part", "vertices", "faces" }`), `same_topology`, and `displacement`
//! (null when the topology differs): `max`, `mean`, `rms`, `moved`, `box` (the moved vertices'
//! bounds, at their places in `a` and `b`, with `min` and `max`; null when none moved) and
//! `along_normal` (`mean`, `min`, `max` along `a`'s vertex normals, positive outward).

use gs_protocol::{ErrorCode, PlainMesh};
use gs_testkit::fixtures::{cube, icosphere};
use gs_testkit::measure::bounds;
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::support::agent;

fn diff(s: &mut Session, a: &str, b: &str) -> Value {
    s.query("mesh.diff", json!({ "a": a, "b": b }))
}

fn number(value: &Value, path: &str) -> f64 {
    let found = path.split('.').fold(value, |v, key| &v[key]);
    found
        .as_f64()
        .unwrap_or_else(|| panic!("no number at {path} in {value}"))
}

fn near(value: &Value, path: &str, want: f64, within: f64) {
    let got = number(value, path);
    assert!(
        (got - want).abs() <= within,
        "{path}: got {got}, want {want} within {within}: {value}"
    );
}

fn scaled(mesh: &PlainMesh, by: f32) -> PlainMesh {
    let mut out = mesh.clone();
    for p in &mut out.positions {
        *p = p.map(|c| c * by);
    }
    out
}

/// A unit icosphere of 642 vertices, made lopsided so no displacement is symmetric by luck.
fn lumpy() -> PlainMesh {
    let mut mesh = icosphere(3);
    for p in &mut mesh.positions {
        p[1] *= 0.7;
        p[2] += 0.2 * p[0] * p[0];
    }
    mesh
}

#[test]
fn a_part_against_itself_is_zero_everywhere() {
    let mut s = agent();
    s.add_part("ball", &lumpy());
    let result = diff(&mut s, "part:ball", "part:ball");
    assert_eq!(result["same_topology"], json!(true), "{result}");
    let d = &result["displacement"];
    for key in ["max", "mean", "rms", "moved"] {
        near(d, key, 0.0, 0.0);
    }
    for key in ["mean", "min", "max"] {
        near(&d["along_normal"], key, 0.0, 0.0);
    }
    assert_eq!(d["box"], Value::Null, "nothing moved, so no box: {result}");
}

#[test]
fn a_translation_moves_every_vertex_by_its_length() {
    let mut s = agent();
    let mesh = lumpy();
    s.add_part("a", &mesh);
    s.add_part("b", &mesh);
    s.ok(
        "mesh.translate",
        json!({ "part": "b", "direction": [1, 0, 0], "distance": 0.1 }),
    );
    let result = diff(&mut s, "part:a", "part:b");
    let d = &result["displacement"];
    near(d, "max", 0.1, 1e-6);
    near(d, "mean", 0.1, 1e-6);
    near(d, "rms", 0.1, 1e-6);
    near(d, "moved", mesh.positions.len() as f64, 0.0);
    // The box spans both ends: a's lowest x, b's highest (0.1 further), y and z unchanged.
    let [min, max] = bounds(&mesh.positions).unwrap_or_else(|| panic!("empty mesh"));
    let corner = |key: &str, axis: usize| {
        (d["box"][key][axis].as_f64()).unwrap_or_else(|| panic!("no box {key}: {result}"))
    };
    let want_max = [max[0] + 0.1, max[1], max[2]];
    for axis in 0..3 {
        let (lo, hi) = (corner("min", axis), corner("max", axis));
        assert!(
            (lo - f64::from(min[axis])).abs() <= 1e-6
                && (hi - f64::from(want_max[axis])).abs() <= 1e-6,
            "box axis {axis}: got {lo}..{hi}, want {}..{}: {result}",
            min[axis],
            want_max[axis]
        );
    }
    // Along the normals, +x is outward on one side and inward on the other.
    assert!(
        number(d, "along_normal.max") > 0.05 && number(d, "along_normal.min") < -0.05,
        "a sideways move goes out on +x and in on -x: {result}"
    );

    // A tighter epsilon counts the same; one above the move counts none.
    let loose = s.query(
        "mesh.diff",
        json!({ "a": "part:a", "b": "part:b", "epsilon": 0.2 }),
    );
    near(&loose["displacement"], "moved", 0.0, 0.0);
    assert_eq!(loose["displacement"]["box"], Value::Null, "{loose}");
}

#[test]
fn growing_a_sphere_moves_it_outward_along_the_normals() {
    let mut s = agent();
    let radius = 2.0;
    let small = scaled(&icosphere(4), radius);
    s.add_part("small", &small);
    s.add_part("big", &scaled(&small, 1.01));
    let grown = diff(&mut s, "part:small", "part:big");
    let along = &grown["displacement"]["along_normal"];
    near(along, "mean", 0.01 * f64::from(radius), 1e-5);
    near(along, "min", 0.01 * f64::from(radius), 1e-4);
    near(
        &grown["displacement"],
        "mean",
        0.01 * f64::from(radius),
        1e-5,
    );
    let shrunk = diff(&mut s, "part:big", "part:small");
    near(
        &shrunk["displacement"]["along_normal"],
        "mean",
        -0.01 * f64::from(radius),
        1e-5,
    );
}

#[test]
fn a_checkpoint_reference_reads_the_checkpoints_part() {
    let mut s = agent();
    let small = icosphere(4);
    s.add_part("ball", &small);
    s.add_part("big", &scaled(&small, 1.01));
    s.ok("history.checkpoint", json!({ "name": "base" }));
    s.ok(
        "mesh.translate",
        json!({ "part": "ball", "direction": [0, 0, 1], "distance": 0.5 }),
    );
    let since = diff(&mut s, "checkpoint:base/ball", "part:ball");
    assert_eq!(since["a"]["checkpoint"], json!("base"), "{since}");
    near(&since["displacement"], "max", 0.5, 1e-6);
    near(&since["displacement"], "mean", 0.5, 1e-6);
    // The checkpoint's normals are its own: its ball grows outward to `big`.
    let grown = diff(&mut s, "checkpoint:base/ball", "part:big");
    near(&grown["displacement"]["along_normal"], "mean", 0.01, 1e-5);
    let same = diff(&mut s, "checkpoint:base/ball", "checkpoint:base/ball");
    near(&same["displacement"], "max", 0.0, 0.0);

    let missing = s.err(
        "mesh.diff",
        json!({ "a": "checkpoint:nope/ball", "b": "part:ball" }),
    );
    assert_eq!(code(missing.code), code(ErrorCode::NotFound), "{missing:?}");
    let unprefixed = s.err("mesh.diff", json!({ "a": "ball", "b": "part:ball" }));
    assert_eq!(
        code(unprefixed.code),
        code(ErrorCode::ArgsInvalid),
        "{unprefixed:?}"
    );
}

/// A checkpoint name cannot hold a `/`, so `checkpoint:NAME/PART` splits right at the first
/// one even when the part's name holds more (review 0007, m2: "a/b" was saved, and
/// `checkpoint:a/b/ball` looked for a checkpoint "a").
#[test]
fn a_checkpoint_name_holds_no_slash_so_a_part_name_may() {
    let mut s = agent();
    s.add_part("wing/left", &icosphere(2));
    let err = s.err("history.checkpoint", json!({ "name": "a/b" }));
    assert_eq!(
        (code(err.code), err.path.as_deref()),
        (code(ErrorCode::ArgsInvalid), Some("/name")),
        "{err:?}"
    );
    assert!(err.message.contains("mesh.diff"), "{}", err.message);
    s.ok("history.checkpoint", json!({ "name": "a" }));
    let same = diff(&mut s, "checkpoint:a/wing/left", "part:wing/left");
    assert_eq!(same["a"]["checkpoint"], json!("a"), "{same}");
    near(&same["displacement"], "max", 0.0, 0.0);
}

#[test]
fn different_topology_is_reported_not_refused() {
    let mut s = agent();
    let ball = icosphere(2);
    s.add_part("ball", &ball);
    s.add_part("box", &cube(2));
    let result = diff(&mut s, "part:ball", "part:box");
    assert_eq!(result["same_topology"], json!(false), "{result}");
    assert_eq!(result["displacement"], Value::Null, "{result}");
    let box_mesh = cube(2);
    for (side, mesh) in [("a", &ball), ("b", &box_mesh)] {
        assert_eq!(
            result[side]["vertices"],
            json!(mesh.positions.len()),
            "{result}"
        );
        assert_eq!(result[side]["faces"], json!(mesh.faces.len()), "{result}");
    }

    // Same counts, faces wound the other way: still not the same topology.
    let mut flipped = ball.clone();
    for face in &mut flipped.faces {
        face.swap(1, 2);
    }
    s.add_part("flipped", &flipped);
    let result = diff(&mut s, "part:ball", "part:flipped");
    assert_eq!(result["same_topology"], json!(false), "{result}");
    assert_eq!(result["displacement"], Value::Null, "{result}");
}
