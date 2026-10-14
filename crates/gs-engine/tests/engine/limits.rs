//! What every part must stay so f32 can hold it: coordinates finite and within ±1e18, and no
//! farther from the origin than 16384 times its size. Making a part that breaks this, or an
//! edit that would, is refused and changes nothing; so is a mesh the mesh store cannot hold.
//! Faces of zero area are legal (review 0002, M2): sculpting and imported files make them.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute};
use serde_json::{Value, json};

use gs_testkit::{code, failure};

use crate::support::{agent, tagged_triangle};

const NO: u32 = u32::MAX;

/// Asserts `err` is an argument error whose message contains `says`, with a hint.
fn refused(err: &gs_protocol::CommandError, says: &str, what: &str) {
    assert_eq!(code(err.code), "E_ARGS_INVALID", "{what}: {}", err.message);
    assert!(err.message.contains(says), "{what}: {}", err.message);
    assert!(err.hint.is_some(), "{what}: says how to fix it");
}

/// Faces of zero area are legal wherever parts are made: the validator does not call them a
/// problem, and making, undoing and redoing such a part is exact.
#[test]
fn zero_area_faces_are_legal_where_parts_are_made() {
    let mut s = agent();
    let flat = json!({ "positions": [[0, 0, 0], [1, 0, 0], [2, 0, 0], [0, 1, 0]],
                       "faces": [[0, 1, 2, NO], [1, 0, 3, NO]] });
    let checked = s.query("mesh.validate", json!({ "mesh": flat }));
    assert_eq!(
        checked,
        json!({ "valid": true, "problems": [] }),
        "a flat face is no problem"
    );
    let empty = s.hash();
    s.ok("part.create", json!({ "name": "flat", "mesh": flat }));
    let made = s.hash();
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), empty, "undo removes it exactly");
    s.ok("history.redo", json!({}));
    assert_eq!(s.hash(), made, "redo brings it back exactly");
}

#[test]
fn parts_stay_near_enough_to_the_origin() {
    let mut s = agent();
    s.ok(
        "primitive.sphere",
        json!({ "name": "a", "subdivisions": 4 }),
    );
    let far = |d: f64| json!({ "part": "a", "direction": [1, 0, 0], "distance": d });
    // Along one axis only, f32 would flatten the sphere into a disc without collapsing a face.
    let err = s.err("mesh.translate", far(1e8));
    refused(&err, "from the origin", "a move f32 cannot hold");
    assert!(
        err.hint.as_deref().is_some_and(|h| h.contains("origin")),
        "{:?}",
        err.hint
    );
    s.ok("mesh.translate", far(1e4));
    s.ok("mesh.translate", far(-1e4));
    let made_and_moved = s.batch(&[
        ("primitive.cube", json!({ "name": "b" })),
        (
            "mesh.translate",
            json!({ "part": "b", "direction": [0, 1, 0], "distance": 1e9 }),
        ),
    ]);
    let err = failure(made_and_moved, "a new part moved too far in the same call");
    refused(&err, "from the origin", "a new part moved too far");
    let parts = s.query("mesh.stats", json!({}))["parts"].clone();
    assert_eq!(parts.as_array().map(Vec::len), Some(1), "no part b");
}

/// The reach is a limit, not a margin: a triangle exactly 16384 times its size (5) from the
/// origin is fine, one unit farther is not.
#[test]
fn the_reach_is_exact() {
    let mut s = agent();
    let edge = json!({ "positions": [[81917, 0, 0], [81920, 0, 0], [81917, 4, 0]],
                       "faces": [[0, 1, 2, NO]] });
    s.ok("part.create", json!({ "name": "edge", "mesh": edge }));
    let along_x = |d: f64| json!({ "direction": [1, 0, 0], "distance": d });
    let err = s.err("mesh.translate", along_x(1.0));
    refused(&err, "from the origin", "one unit past the reach");
    s.ok("mesh.translate", along_x(-10.0));
}

/// An edit that flattens a face is legal too: a tiny triangle beside a large one, rounded flat
/// by a move to 1000 units out, moves, and undo restores it exactly.
#[test]
fn edits_that_flatten_faces_are_legal_and_undo_exactly() {
    let mut s = agent();
    let tiny = 1e-6;
    let mesh = json!({ "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0],
                                     [0.5, 0.5, 0], [0.5 + tiny, 0.5, 0], [0.5, 0.5 + tiny, 0]],
                       "faces": [[0, 1, 2, NO], [3, 4, 5, NO]] });
    s.ok("part.create", json!({ "name": "t", "mesh": mesh }));
    let before = s.hash();
    s.ok(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 1000 }),
    );
    s.ok("history.undo", json!({}));
    assert_eq!(
        s.hash(),
        before,
        "undo restores the unflattened triangle exactly"
    );
}

#[test]
fn coordinates_stay_finite_and_within_range() {
    let mut s = agent();
    let triangle = |far: f64| {
        json!({ "positions": [[0, 0, 0], [far, 0, 0], [0, far, 0]],
                                      "faces": [[0, 1, 2, NO]] })
    };
    s.ok(
        "part.create",
        json!({ "name": "big", "mesh": triangle(1e18) }),
    );
    let along_x = |d: f64| json!({ "part": "big", "direction": [1, 0, 0], "distance": d });
    for d in [1e18, 3e38] {
        let err = s.err("mesh.translate", along_x(d));
        refused(&err, "out of range", "a move past 1e18");
    }
    s.ok("mesh.translate", along_x(-1e18));
    let err = s.err("mesh.translate", along_x(-1e18));
    refused(&err, "out of range", "a move past -1e18");
    let err = s.err(
        "part.create",
        json!({ "name": "huge", "mesh": triangle(2e18) }),
    );
    refused(&err, "out of range", "a part made past 1e18");
}

/// A square fan of 256 triangles around one center vertex: valid, but the store holds at most
/// 255 faces around a vertex.
fn fan() -> Value {
    let steps = (0..64).map(|k| -1.0 + f64::from(k) / 32.0);
    let right = steps.clone().map(|t| [1.0, t]);
    let top = steps.clone().map(|t| [-t, 1.0]);
    let left = steps.clone().map(|t| [-1.0, -t]);
    let bottom = steps.map(|t| [t, -1.0]);
    let rim: Vec<[f64; 3]> = (right.chain(top).chain(left).chain(bottom))
        .map(|[x, y]| [x, y, 0.0])
        .collect();
    let positions: Vec<[f64; 3]> = std::iter::once([0.0; 3]).chain(rim).collect();
    let faces: Vec<[u32; 4]> = (1..=256u32).map(|i| [0, i, i % 256 + 1, NO]).collect();
    json!({ "positions": positions, "faces": faces })
}

#[test]
fn meshes_the_store_cannot_hold_are_refused() {
    let mut s = agent();
    let mesh = fan();
    let valid = s.query("mesh.validate", json!({ "mesh": mesh }))["valid"].clone();
    assert_eq!(valid, json!(true), "the fan itself is fine");
    let err = s.err("part.create", json!({ "name": "fan", "mesh": mesh }));
    assert_eq!(code(err.code), "E_ARGS_INVALID", "{}", err.message);
    assert_eq!(err.path.as_deref(), Some("/mesh"), "{}", err.message);
    assert!(err.message.contains("255"), "{}", err.message);
    let mut typed = tagged_triangle();
    let ints = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::U32(vec![1; 3]),
    };
    typed.attributes = BTreeMap::from([("mask".to_string(), ints)]);
    let err = s.err("part.create", json!({ "name": "t", "mesh": typed }));
    assert_eq!(code(err.code), "E_ARGS_INVALID", "{}", err.message);
    assert!(
        err.message.contains("mask"),
        "names the column: {}",
        err.message
    );
}
