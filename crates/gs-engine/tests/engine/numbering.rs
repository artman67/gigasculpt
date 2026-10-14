//! Parts keep the vertex and face numbering they were made with, although the mesh store sorts
//! vertices in space: what goes into `part.create` comes back out of `part.get` unchanged, and
//! edits and undo keep it.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use serde_json::json;

use crate::support::agent;

/// A tetrahedron listed far from spatial order, with faces starting at varied corners and
/// columns named like the store's own temporary ones.
fn scrambled() -> PlainMesh {
    let column = |domain, values| Attribute { domain, values };
    let attributes = BTreeMap::from([
        (
            "given vertex".to_string(),
            column(AttrDomain::Vertex, AttrValues::U32(vec![9, 8, 7, 6])),
        ),
        (
            "given face".to_string(),
            column(AttrDomain::Face, AttrValues::F32(vec![0.5, 1.5, 2.5, 3.5])),
        ),
        (
            "mask".to_string(),
            column(
                AttrDomain::Vertex,
                AttrValues::F32(vec![0.0, 0.25, 0.5, 1.0]),
            ),
        ),
    ]);
    PlainMesh {
        positions: vec![
            [0.0, 0.0, 4.0],
            [4.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 4.0, 0.0],
        ],
        faces: vec![
            [3, 1, 2, u32::MAX],
            [2, 1, 0, u32::MAX],
            [0, 3, 2, u32::MAX],
            [1, 3, 0, u32::MAX],
        ],
        attributes,
    }
}

#[test]
fn a_part_comes_back_exactly_as_it_was_made() {
    let mut s = agent();
    let mesh = scrambled();
    s.ok("part.create", json!({ "name": "t", "mesh": mesh }));
    assert_eq!(json!(s.mesh("t")), json!(mesh), "part.get gives it back");
    let names = s.query("mesh.stats", json!({}))["parts"][0]["attributes"].clone();
    assert_eq!(
        names,
        json!(["given face", "given vertex", "mask"]),
        "no extra columns"
    );
    s.ok(
        "mesh.translate",
        json!({ "direction": [0, 0, 1], "distance": 2 }),
    );
    let mut moved = mesh.clone();
    for p in &mut moved.positions {
        p[2] += 2.0;
    }
    assert_eq!(
        json!(s.mesh("t")),
        json!(moved),
        "a move keeps the numbering"
    );
    s.ok("history.checkpoint", json!({ "name": "moved" }));
    s.ok("history.undo", json!({}));
    assert_eq!(json!(s.mesh("t")), json!(mesh), "so does undo");
    s.ok("history.restore", json!({ "name": "moved" }));
    assert_eq!(json!(s.mesh("t")), json!(moved), "and a restore");
}
